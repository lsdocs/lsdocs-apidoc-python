//! Python source to API items, statically (no code runs), with ruff's
//! parser.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lsdocs_apidoc::{Item, ItemKind, Location, Param, ParamKind, Signature, Visibility};
use lsdocs_plugin_sdk::protocol::{Diagnostic, Severity};
use ruff_python_ast::{Expr, Parameters, Stmt, StmtClassDef, StmtFunctionDef};
use ruff_text_size::{Ranged, TextRange};

/// Values longer than this are cut (`...`) in signatures.
const MAX_VALUE: usize = 80;

/// What extraction found.
#[derive(Debug, Default)]
pub(crate) struct Found {
    pub items: Vec<Item>,
    pub files: Vec<String>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Extracts the packages and modules at `paths` (relative to `root`): a
/// directory is a package (its `__init__` and every module below it), a
/// `.py` or `.pyi` file a module.
pub(crate) fn extract(paths: &[String], root: &Path) -> Found {
    let mut found = Found::default();
    for path in paths {
        let full = root.join(path);
        let name = full
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if full.is_dir() {
            if let Some(item) = package(&full, vec![name], path, &mut found) {
                found.items.push(item);
            }
        } else if full.is_file() {
            if let Some(item) = module_file(&full, vec![name], path, ItemKind::Module, &mut found) {
                found.items.push(item);
            }
        } else {
            found.diagnostics.push(problem(
                path,
                None,
                format!("no such file or directory: {path}"),
            ));
        }
    }
    found
}

fn problem(file: &str, line: Option<u32>, message: String) -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        message: match line {
            Some(line) => format!("{file}:{line}: {message}"),
            None => message,
        },
        code: Some("apidoc-python".into()),
        ..Diagnostic::default()
    }
}

/// The module source for `stem` in `dir`: the stub if there is one.
fn module_source(dir: &Path, stem: &str) -> Option<PathBuf> {
    ["pyi", "py"]
        .iter()
        .map(|ext| dir.join(format!("{stem}.{ext}")))
        .find(|p| p.is_file())
}

/// A package directory: its `__init__` as the package's own module, and
/// its modules and subpackages as members.
fn package(dir: &Path, path: Vec<String>, shown: &str, found: &mut Found) -> Option<Item> {
    let mut item = match module_source(dir, "__init__") {
        Some(init) => {
            let shown = format!("{shown}/{}", init.file_name()?.to_string_lossy());
            module_file(&init, path.clone(), &shown, ItemKind::Package, found)?
        }
        None => Item::new(ItemKind::Package, path.clone()),
    };
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    let mut seen = BTreeSet::new();
    for entry in entries {
        let Some(name) = entry.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let shown_entry = format!("{shown}/{name}");
        if entry.is_dir() {
            let is_package = module_source(&entry, "__init__").is_some();
            if is_package && !name.starts_with(['.', '_']) {
                let mut sub = path.clone();
                sub.push(name);
                if let Some(child) = package(&entry, sub, &shown_entry, found) {
                    item.members.push(child);
                }
            }
            continue;
        }
        let Some(stem) = name
            .strip_suffix(".pyi")
            .or_else(|| name.strip_suffix(".py"))
        else {
            continue;
        };
        if stem == "__init__"
            || stem.starts_with('_') && !stem.starts_with("__")
            || !seen.insert(stem.to_owned())
        {
            continue;
        }
        let Some(source) = module_source(dir, stem) else {
            continue;
        };
        let mut sub = path.clone();
        sub.push(stem.to_owned());
        let shown = format!("{shown}/{}", source.file_name()?.to_string_lossy());
        if let Some(child) = module_file(&source, sub, &shown, ItemKind::Module, found) {
            item.members.push(child);
        }
    }
    Some(item)
}

/// One module file.
fn module_file(
    file: &Path,
    path: Vec<String>,
    shown: &str,
    kind: ItemKind,
    found: &mut Found,
) -> Option<Item> {
    found.files.push(shown.to_owned());
    let source = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) => {
            found
                .diagnostics
                .push(problem(shown, None, format!("cannot read: {e}")));
            return None;
        }
    };
    let parsed = match ruff_python_parser::parse_module(&source) {
        Ok(parsed) => parsed,
        Err(e) => {
            let line = line_of(&source, e.location);
            found.diagnostics.push(problem(
                shown,
                Some(line),
                format!("syntax error: {}", e.error),
            ));
            return None;
        }
    };
    let cx = Cx {
        source: &source,
        file: shown,
    };
    let body = &parsed.syntax().body;
    let mut item = Item::new(kind, path.clone());
    item.docs = docstring(body);
    item.location = Some(Location {
        file: shown.to_owned(),
        line: None,
    });
    let all = dunder_all(body);
    item.members = cx.body(body, &path, Scope::Module);
    if let Some(all) = all {
        for member in &mut item.members {
            if !all.contains(member.name()) {
                member.visibility = Visibility::Private;
            }
        }
    }
    Some(item)
}

/// The line (from 1) of `range`'s start.
fn line_of(source: &str, range: TextRange) -> u32 {
    let at = usize::from(range.start()).min(source.len());
    let line = source[..at].bytes().filter(|b| *b == b'\n').count() + 1;
    u32::try_from(line).unwrap_or(u32::MAX)
}

/// A body's docstring: a string literal as its first statement.
fn docstring(body: &[Stmt]) -> Option<String> {
    match body.first()? {
        Stmt::Expr(e) => string(&e.value),
        _ => None,
    }
}

fn string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::StringLiteral(s) => Some(s.value.to_str().to_owned()),
        _ => None,
    }
}

/// `__all__ = [...]` (or a tuple) of string literals, if the module has it.
fn dunder_all(body: &[Stmt]) -> Option<BTreeSet<String>> {
    for stmt in body {
        let (targets, value): (Vec<&Expr>, &Expr) = match stmt {
            Stmt::Assign(a) => (a.targets.iter().collect(), &a.value),
            Stmt::AnnAssign(a) => match &a.value {
                Some(value) => (vec![&a.target], value),
                None => continue,
            },
            _ => continue,
        };
        if !targets
            .iter()
            .any(|t| matches!(t, Expr::Name(n) if n.id.as_str() == "__all__"))
        {
            continue;
        }
        let elements: &[Expr] = match value {
            Expr::List(l) => &l.elts,
            Expr::Tuple(t) => &t.elts,
            _ => continue,
        };
        return Some(elements.iter().filter_map(string).collect());
    }
    None
}

/// What a body belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Module,
    Class,
    /// An enum class: its assignments are variants.
    Enum,
}

struct Cx<'a> {
    source: &'a str,
    file: &'a str,
}

/// The visibility a name has by Python's conventions: `_name` is
/// private, `__dunder__` public.
fn visibility(name: &str) -> Visibility {
    if name.starts_with('_') && !(name.starts_with("__") && name.ends_with("__")) {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

impl Cx<'_> {
    fn text(&self, node: &impl Ranged) -> String {
        self.source[node.range()].to_owned()
    }

    /// An expression's text, cut if long (for values).
    fn short(&self, node: &impl Ranged) -> String {
        let text = self.text(node);
        let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() > MAX_VALUE {
            let cut: String = flat.chars().take(MAX_VALUE).collect();
            format!("{cut}...")
        } else {
            flat
        }
    }

    fn location(&self, node: &impl Ranged) -> Option<Location> {
        Some(Location {
            file: self.file.to_owned(),
            line: Some(line_of(self.source, node.range())),
        })
    }

    /// The items a body defines.
    fn body(&self, body: &[Stmt], path: &[String], scope: Scope) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        for (i, stmt) in body.iter().enumerate() {
            // A string right after an assignment documents it (Sphinx's
            // attribute docstrings).
            let next_doc = body.get(i + 1).and_then(|s| match s {
                Stmt::Expr(e) => string(&e.value),
                _ => None,
            });
            let made = match stmt {
                Stmt::FunctionDef(f) => Some(self.function(f, path, scope)),
                Stmt::ClassDef(c) => Some(self.class(c, path)),
                Stmt::AnnAssign(a) => match &*a.target {
                    Expr::Name(name) => {
                        let ty = self.text(&*a.annotation);
                        let kind = if ty.ends_with("TypeAlias") {
                            ItemKind::TypeAlias
                        } else {
                            data_kind(name.id.as_str(), scope)
                        };
                        let mut item = self.named(kind.clone(), path, name.id.as_str(), stmt);
                        if kind == ItemKind::TypeAlias {
                            item.ty = a.value.as_ref().map(|v| self.short(&**v));
                        } else {
                            item.ty = Some(ty);
                            item.value = a.value.as_ref().map(|v| self.short(&**v));
                        }
                        item.docs = next_doc;
                        Some(item)
                    }
                    _ => None,
                },
                Stmt::Assign(a) => match a.targets.as_slice() {
                    [Expr::Name(name)] if name.id.as_str() != "__all__" => {
                        let mut item = self.named(
                            data_kind(name.id.as_str(), scope),
                            path,
                            name.id.as_str(),
                            stmt,
                        );
                        item.value = Some(self.short(&*a.value));
                        item.docs = next_doc;
                        Some(item)
                    }
                    _ => None,
                },
                Stmt::TypeAlias(t) => match &*t.name {
                    Expr::Name(name) => {
                        let mut item =
                            self.named(ItemKind::TypeAlias, path, name.id.as_str(), stmt);
                        item.ty = Some(self.short(&*t.value));
                        if let Some(params) = &t.type_params {
                            item.signature = Some(Signature {
                                generics: params.iter().map(|p| self.text(p)).collect(),
                                ..Signature::default()
                            });
                        }
                        item.docs = next_doc;
                        Some(item)
                    }
                    _ => None,
                },
                _ => None,
            };
            let Some(item) = made else {
                continue;
            };
            // A later definition replaces an earlier one (overloads in
            // stubs keep the first, documented one).
            match items.iter_mut().find(|i| i.path == item.path) {
                Some(existing)
                    if is_overload(stmt) || existing.docs.is_some() && item.docs.is_none() => {}
                Some(existing) => *existing = item,
                None => items.push(item),
            }
        }
        items
    }

    fn named(&self, kind: ItemKind, path: &[String], name: &str, node: &impl Ranged) -> Item {
        let mut full = path.to_vec();
        full.push(name.to_owned());
        let mut item = Item::new(kind, full);
        item.visibility = visibility(name);
        item.location = self.location(node);
        item
    }

    fn decorators(&self, f: &StmtFunctionDef) -> Vec<String> {
        f.decorator_list
            .iter()
            .map(|d| self.text(&d.expression))
            .collect()
    }

    fn function(&self, f: &StmtFunctionDef, path: &[String], scope: Scope) -> Item {
        let decorators = self.decorators(f);
        let has = |name: &str| {
            decorators
                .iter()
                .any(|d| d == name || d.ends_with(&format!(".{name}")))
        };
        let in_class = scope != Scope::Module;
        let kind = if in_class && (has("property") || has("cached_property")) {
            ItemKind::Property
        } else if in_class {
            ItemKind::Method
        } else {
            ItemKind::Function
        };
        let mut item = self.named(kind.clone(), path, f.name.as_str(), f);
        item.docs = docstring(&f.body);
        if kind == ItemKind::Property {
            item.ty = f.returns.as_ref().map(|r| self.text(&**r));
            return item;
        }
        let mut modifiers = Vec::new();
        if f.is_async {
            modifiers.push("async".to_owned());
        }
        for special in ["classmethod", "staticmethod", "abstractmethod"] {
            if has(special) {
                modifiers.push(special.to_owned());
            }
        }
        let receiver = in_class && !has("staticmethod");
        item.signature = Some(Signature {
            generics: f
                .type_params
                .as_ref()
                .map(|p| p.iter().map(|t| self.text(t)).collect())
                .unwrap_or_default(),
            params: self.params(&f.parameters, receiver),
            returns: f.returns.as_ref().map(|r| self.text(&**r)),
            modifiers,
            text: None,
        });
        let other: Vec<String> = decorators
            .into_iter()
            .filter(|d| {
                !["classmethod", "staticmethod", "abstractmethod", "overload"].contains(&d.as_str())
            })
            .collect();
        if !other.is_empty() {
            item.extra.insert("decorators".into(), other.join(", "));
        }
        item
    }

    fn params(&self, p: &Parameters, receiver: bool) -> Vec<Param> {
        let mut out = Vec::new();
        let with_default = |p: &ruff_python_ast::ParameterWithDefault, kind: ParamKind| Param {
            name: p.parameter.name.as_str().to_owned(),
            ty: p.parameter.annotation.as_ref().map(|a| self.text(&**a)),
            default: p.default.as_ref().map(|d| self.short(&**d)),
            kind,
        };
        for a in &p.posonlyargs {
            out.push(with_default(a, ParamKind::PositionalOnly));
        }
        for a in &p.args {
            out.push(with_default(a, ParamKind::Normal));
        }
        if let Some(v) = &p.vararg {
            out.push(Param {
                name: v.name.as_str().to_owned(),
                ty: v.annotation.as_ref().map(|a| self.text(&**a)),
                default: None,
                kind: ParamKind::Variadic,
            });
        }
        for a in &p.kwonlyargs {
            out.push(with_default(a, ParamKind::KeywordOnly));
        }
        if let Some(k) = &p.kwarg {
            out.push(Param {
                name: k.name.as_str().to_owned(),
                ty: k.annotation.as_ref().map(|a| self.text(&**a)),
                default: None,
                kind: ParamKind::KeywordVariadic,
            });
        }
        if receiver && let Some(first) = out.first_mut() {
            first.kind = ParamKind::Receiver;
        }
        out
    }

    fn class(&self, c: &StmtClassDef, path: &[String]) -> Item {
        let bases: Vec<String> = c
            .arguments
            .as_ref()
            .map(|a| a.args.iter().map(|b| self.text(b)).collect())
            .unwrap_or_default();
        let is_enum = bases
            .iter()
            .any(|b| b.ends_with("Enum") || b.ends_with("Flag"));
        let kind = if is_enum {
            ItemKind::Enum
        } else {
            ItemKind::Class
        };
        let mut item = self.named(kind, path, c.name.as_str(), c);
        item.docs = docstring(&c.body);
        item.bases = bases;
        let scope = if is_enum { Scope::Enum } else { Scope::Class };
        let mut members = self.body(&c.body, &item.path, scope);
        // The constructor's parameters are the class's signature, and its
        // `self.x: T` assignments (documented ones too) its fields.
        let init = c.body.iter().find_map(|s| match s {
            Stmt::FunctionDef(f) if f.name.as_str() == "__init__" => Some(f),
            _ => None,
        });
        let mut generics: Vec<String> = c
            .type_params
            .as_ref()
            .map(|p| p.iter().map(|t| self.text(t)).collect())
            .unwrap_or_default();
        if let Some(init) = init {
            let mut params = self.params(&init.parameters, true);
            params.retain(|p| p.kind != ParamKind::Receiver);
            item.signature = Some(Signature {
                generics: std::mem::take(&mut generics),
                params,
                ..Signature::default()
            });
            if item.docs.is_none() {
                item.docs = docstring(&init.body);
            }
            for (i, stmt) in init.body.iter().enumerate() {
                let (target, ty, value) = match stmt {
                    Stmt::AnnAssign(a) => (
                        &*a.target,
                        Some(self.text(&*a.annotation)),
                        a.value.as_ref().map(|v| self.short(&**v)),
                    ),
                    Stmt::Assign(a) if a.targets.len() == 1 => (&a.targets[0], None, None),
                    _ => continue,
                };
                let Expr::Attribute(attr) = target else {
                    continue;
                };
                if !matches!(&*attr.value, Expr::Name(n) if n.id.as_str() == "self") {
                    continue;
                }
                let doc = init.body.get(i + 1).and_then(|s| match s {
                    Stmt::Expr(e) => string(&e.value),
                    _ => None,
                });
                // Annotated or documented ones only, as autodoc does.
                if ty.is_none() && doc.is_none() {
                    continue;
                }
                let name = attr.attr.as_str();
                if members.iter().any(|m| m.name() == name) {
                    continue;
                }
                let mut field = self.named(ItemKind::Field, &item.path, name, stmt);
                field.ty = ty;
                field.value = value;
                field.docs = doc;
                members.push(field);
            }
        }
        if !generics.is_empty() {
            item.signature = Some(Signature {
                generics,
                ..Signature::default()
            });
        }
        // `__init__` is the class's signature; other special methods are
        // private unless documented.
        members.retain(|m| m.name() != "__init__");
        for member in &mut members {
            let name = member.name().to_owned();
            if name.starts_with("__") && name.ends_with("__") && member.docs.is_none() {
                member.visibility = Visibility::Private;
            }
        }
        item.members = members;
        item
    }
}

fn data_kind(name: &str, scope: Scope) -> ItemKind {
    match scope {
        Scope::Enum => ItemKind::Variant,
        Scope::Class => ItemKind::Field,
        Scope::Module
            if name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') =>
        {
            ItemKind::Constant
        }
        Scope::Module => ItemKind::Variable,
    }
}

fn is_overload(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::FunctionDef(f) => f.decorator_list.iter().any(|d| match &d.expression {
            Expr::Name(n) => n.id.as_str() == "overload",
            Expr::Attribute(a) => a.attr.as_str() == "overload",
            _ => false,
        }),
        _ => false,
    }
}
