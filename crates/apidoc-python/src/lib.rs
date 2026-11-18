//! First-party plugin documenting Python APIs from their source code.
//!
//! Reads Python packages and modules statically, with ruff's parser (no
//! code runs, nothing is imported), and documents them with the API
//! framework (`lsdocs-apidoc-plugin`): a page per module under `api/py`,
//! the `py` domain's directives and roles, references between them.
//!
//! ```toml
//! [[plugins]]
//! name = "python"
//! builtin = "apidoc-python"
//! ```
//!
//! with its settings in `.config/lsdocs/python/settings.toml`:
//!
//! ```toml
//! paths = ["src/mypkg"]
//! ```
//!
//! - **Paths** (relative to the site directory): a directory is a package
//!   (`__init__.py`, its modules and subpackages; names starting with `_`
//!   are skipped), a `.py` or `.pyi` file a module. A stub (`.pyi`) is read
//!   instead of the module next to it.
//! - **What is documented:** modules, classes (an `Enum` subclass is an
//!   enum, its assignments variants), functions and methods (async,
//!   `classmethod`, `staticmethod`; the first overload of a stub),
//!   properties, module and class variables (upper-case names are
//!   constants) with the docstring after them, `self.x` attributes
//!   assigned in `__init__` with an annotation or a docstring, and type
//!   aliases (`type X = ...`, `X: TypeAlias = ...`). A class's signature
//!   is its `__init__`'s.
//! - **Private items:** names starting with `_` (but not `__dunder__`
//!   ones), names a module's `__all__` leaves out, and undocumented
//!   special methods. The `private` option shows them.
//! - **Docstrings** are reStructuredText by default (`doc_format = "md"`
//!   for Markdown), with Google, NumPy or Sphinx field list sections.
//! - **The `py` domain:** `py:function`, `py:class`, `py:method`,
//!   `py:classmethod`, `py:staticmethod`, `py:attribute`, `py:property`,
//!   `py:data`, `py:exception`, `py:decorator`, `py:type`, `py:module`,
//!   `py:currentmodule`, `py:auto`; roles `py:func`, `py:meth`,
//!   `py:class`, `py:attr`, `py:data`, `py:const`, `py:exc`, `py:mod`,
//!   `py:type`, `py:obj`.
//!
//! Settings: `paths` (required for generated pages) and those of the
//! framework (`output`, `private`, `title`, `doc_format`, `doc_style`,
//! `generate`).

mod extract;
#[cfg(test)]
mod tests;

use std::path::Path;

use lsdocs_apidoc::{DocStyle, Item, ItemKind, ParamKind};
use lsdocs_apidoc_plugin::{Extracted, Language, default_describe};
use lsdocs_plugin_sdk::Plugin;
use serde_json::Value;

/// The plugin, as registered with the host.
pub fn plugin() -> Plugin {
    lsdocs_apidoc_plugin::plugin(Python, env!("CARGO_PKG_VERSION"))
}

/// Python.
#[derive(Debug, Clone, Copy, Default)]
pub struct Python;

impl Language for Python {
    fn domain(&self) -> &str {
        "py"
    }

    fn name(&self) -> &str {
        "Python"
    }

    fn code_language(&self) -> &str {
        "python"
    }

    fn settings(&self) -> &[(&str, lsdocs_plugin_sdk::config::Kind)] {
        use lsdocs_plugin_sdk::config::Kind;
        &[("paths", Kind::Array)]
    }

    fn doc_format(&self) -> &str {
        "rst"
    }

    fn doc_style(&self) -> DocStyle {
        DocStyle::Auto
    }

    fn extract(&self, options: &Value, root: &Path) -> Result<Extracted, String> {
        let paths: Vec<String> = match options.get("paths") {
            Some(paths) => serde_json::from_value(paths.clone())
                .map_err(|e| format!("`paths` must be a list of paths: {e}"))?,
            None => return Err("the `paths` option is missing".into()),
        };
        let found = extract::extract(&paths, root);
        Ok(Extracted {
            items: found.items,
            files: found.files,
            diagnostics: found.diagnostics,
        })
    }

    fn signature(&self, item: &Item) -> String {
        signature(item)
    }

    fn objects(&self) -> Vec<(String, ItemKind)> {
        [
            ("function", ItemKind::Function),
            ("class", ItemKind::Class),
            ("method", ItemKind::Method),
            ("classmethod", ItemKind::Method),
            ("staticmethod", ItemKind::Method),
            ("attribute", ItemKind::Field),
            ("property", ItemKind::Property),
            ("data", ItemKind::Variable),
            ("exception", ItemKind::Class),
            ("decorator", ItemKind::Function),
            ("type", ItemKind::TypeAlias),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect()
    }

    fn roles(&self) -> Vec<(String, Vec<ItemKind>)> {
        use ItemKind::*;
        [
            ("func", vec![Function, Method]),
            ("meth", vec![Method, Function]),
            ("class", vec![Class, Enum]),
            ("exc", vec![Class]),
            ("attr", vec![Field, Property, Variant, Variable, Constant]),
            ("data", vec![Variable, Constant]),
            ("const", vec![Constant, Variable, Variant]),
            ("mod", vec![Module, Package]),
            ("type", vec![TypeAlias, Class, Enum]),
            ("obj", vec![]),
        ]
        .into_iter()
        .map(|(n, k)| (n.to_owned(), k))
        .collect()
    }

    fn describe(&self, kind: ItemKind, signature: &str, scope: &[String]) -> Option<Item> {
        // Python's generics are in brackets: `f[T](x: T)`.
        let signature = match signature.find('[') {
            Some(open)
                if signature[..open]
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '.') =>
            {
                let close = signature[open..].find(']').map(|i| open + i)?;
                format!(
                    "{}<{}>{}",
                    &signature[..open],
                    &signature[open + 1..close],
                    &signature[close + 1..]
                )
            }
            _ => signature.to_owned(),
        };
        let mut item = default_describe(kind, &signature, scope, ".")?;
        // Parameter kinds from `/` and `*`, which the generic reading drops.
        if let Some(sig) = &mut item.signature
            && let Some(open) = signature.find('(')
        {
            let mut depth = 0;
            let mut parts = vec![String::new()];
            for c in signature[open + 1..].chars() {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' if depth == 0 => break,
                    ')' | ']' | '}' => depth -= 1,
                    ',' if depth == 0 => {
                        parts.push(String::new());
                        continue;
                    }
                    _ => {}
                }
                if let Some(last) = parts.last_mut() {
                    last.push(c);
                }
            }
            let marks: Vec<&str> = parts
                .iter()
                .map(|p| p.trim())
                .filter(|p| !p.is_empty())
                .collect();
            let slash = marks.iter().position(|p| *p == "/");
            let star = marks
                .iter()
                .position(|p| *p == "*" || (p.starts_with('*') && !p.starts_with("**")));
            let mut index = 0;
            for (at, mark) in marks.iter().enumerate() {
                if *mark == "/" || *mark == "*" {
                    continue;
                }
                if let Some(param) = sig.params.get_mut(index) {
                    if slash.is_some_and(|s| at < s) && param.kind == ParamKind::Normal {
                        param.kind = ParamKind::PositionalOnly;
                    } else if star.is_some_and(|s| at > s) && param.kind == ParamKind::Normal {
                        param.kind = ParamKind::KeywordOnly;
                    }
                }
                index += 1;
            }
        }
        Some(item)
    }
}

/// A Python signature: `async classmethod name[T](a: int = 1, /, b, *,
/// c) -> R`, `class Name[T](Base)`, `name: T = value`, `type X = int`.
pub fn signature(item: &Item) -> String {
    let name = item.name();
    let generics = item
        .signature
        .as_ref()
        .filter(|s| !s.generics.is_empty())
        .map(|s| format!("[{}]", s.generics.join(", ")))
        .unwrap_or_default();
    let data = |prefix: &str| {
        let mut out = format!("{prefix}{name}");
        if let Some(ty) = &item.ty {
            out.push_str(": ");
            out.push_str(ty);
        }
        if let Some(value) = &item.value {
            out.push_str(" = ");
            out.push_str(value);
        }
        out
    };
    match &item.kind {
        kind if kind.is_module() => format!("{} {}", kind.label(), item.qualified(".")),
        ItemKind::Class | ItemKind::Enum => {
            let mut out = format!("class {name}{generics}");
            let params = item
                .signature
                .as_ref()
                .map(|s| params(&s.params))
                .filter(|p| !p.is_empty());
            match params {
                Some(params) => out.push_str(&format!("({params})")),
                None if !item.bases.is_empty() => {
                    out.push_str(&format!("({})", item.bases.join(", ")))
                }
                None => {}
            }
            out
        }
        ItemKind::Function | ItemKind::Method => {
            let sig = item.signature.clone().unwrap_or_default();
            let mut out = String::new();
            for modifier in &sig.modifiers {
                out.push_str(modifier);
                out.push(' ');
            }
            out.push_str(name);
            out.push_str(&generics);
            out.push('(');
            out.push_str(&params(&sig.params));
            out.push(')');
            if let Some(returns) = &sig.returns {
                out.push_str(" -> ");
                out.push_str(returns);
            }
            out
        }
        ItemKind::Property => data("property "),
        ItemKind::TypeAlias => match &item.ty {
            Some(ty) => format!("type {name}{generics} = {ty}"),
            None => format!("type {name}{generics}"),
        },
        _ => data(""),
    }
}

/// Parameters with Python's `/` and `*` markers.
fn params(params: &[lsdocs_apidoc::Param]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut positional_only = false;
    let mut starred = false;
    for p in params {
        if positional_only && p.kind != ParamKind::PositionalOnly && p.kind != ParamKind::Receiver {
            out.push("/".into());
            positional_only = false;
        }
        if p.kind == ParamKind::PositionalOnly {
            positional_only = true;
        }
        if p.kind == ParamKind::KeywordOnly && !starred {
            out.push("*".into());
            starred = true;
        }
        let mut text = match p.kind {
            ParamKind::Variadic => {
                starred = true;
                format!("*{}", p.name)
            }
            ParamKind::KeywordVariadic => format!("**{}", p.name),
            _ => p.name.clone(),
        };
        if let Some(ty) = &p.ty {
            text.push_str(": ");
            text.push_str(ty);
        }
        if let Some(default) = &p.default {
            text.push_str(if p.ty.is_some() { " = " } else { "=" });
            text.push_str(default);
        }
        out.push(text);
    }
    if positional_only {
        out.push("/".into());
    }
    out.join(", ")
}
