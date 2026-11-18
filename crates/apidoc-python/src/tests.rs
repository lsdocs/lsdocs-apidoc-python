#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};

use lsdocs_apidoc::{Item, ItemKind, Visibility};
use lsdocs_apidoc_plugin::Language;
use lsdocs_plugin_sdk::doctree::{Node, plain_text};
use lsdocs_plugin_sdk::protocol::{InitializeParams, SiteInfo};
use lsdocs_plugin_sdk::testing::{context_with_parsers, run_transform_in};
use serde_json::json;

use super::{Python, plugin, signature};

/// A fresh directory with `files` in it.
fn site(label: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("lsdocs-py-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (path, text) in files {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}

const INIT: &str = r#""""The *shapes* package.

Use :py:class:`Circle`.
"""

__all__ = ["Circle", "area", "Color", "ORIGIN", "Point"]

from .util import helper

ORIGIN: tuple[int, int] = (0, 0)
"""Where everything starts."""

type Point = tuple[float, float]

hidden = 1


class Color(Enum):
    """Colors."""

    RED = 1
    """Warm."""
    BLUE = 2


class Circle(Shape):
    """A circle.

    Args:
        radius: How big, see :py:func:`area`.
    """

    def __init__(self, radius: float = 1.0, /, *, name: str | None = None) -> None:
        self.radius: float = radius
        self.name = name
        """Its name."""
        self._cache = None

    @property
    def diameter(self) -> float:
        """Twice the radius."""
        return 2 * self.radius

    @classmethod
    def unit(cls) -> "Circle":
        return cls()

    async def grow[T](self, by: T, *more: T, **opts) -> None:
        """Grow it.

        :param by: How much.
        :raises ValueError: If negative.
        """

    def _private(self): ...

    def __repr__(self): ...


def area(shape: Circle) -> float:
    """The area of a :py:class:`~shapes.Circle`."""
"#;

fn extract(files: &[(&str, &str)], paths: &[&str]) -> (Vec<Item>, Vec<String>, Vec<String>) {
    let dir = site("x", files);
    let found = super::extract::extract(
        &paths.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>(),
        &dir,
    );
    let _ = std::fs::remove_dir_all(&dir);
    let problems = found
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect();
    (found.items, found.files, problems)
}

fn find<'a>(items: &'a [Item], path: &str) -> &'a Item {
    items
        .iter()
        .flat_map(Item::walk)
        .find(|i| i.qualified(".") == path)
        .unwrap_or_else(|| panic!("no {path}"))
}

#[test]
fn extracts_packages_modules_and_members() {
    let (items, files, problems) = extract(
        &[
            ("src/shapes/__init__.py", INIT),
            (
                "src/shapes/util.py",
                "def helper(x):\n    \"\"\"Help.\"\"\"\n",
            ),
            ("src/shapes/_internal.py", "x = 1\n"),
            (
                "src/shapes/geo/__init__.pyi",
                "def dist(a: float, b: float) -> float: ...\n",
            ),
            ("src/shapes/geo/__init__.py", "this is not read\n"),
            (
                "src/shapes/stub.pyi",
                "from typing import overload\n@overload\ndef f(x: int) -> int:\n    \"\"\"Doc.\"\"\"\n@overload\ndef f(x: str) -> str: ...\n",
            ),
            ("src/broken.py", "def f(:\n"),
        ],
        &["src/shapes", "src/broken.py"],
    );
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(
        problems[0].starts_with("src/broken.py:1: syntax error"),
        "{problems:?}"
    );
    assert_eq!(
        files,
        [
            "src/shapes/__init__.py",
            "src/shapes/geo/__init__.pyi",
            "src/shapes/stub.pyi",
            "src/shapes/util.py",
            "src/broken.py"
        ]
    );
    let pkg = &items[0];
    assert_eq!(pkg.kind, ItemKind::Package);
    assert!(
        pkg.docs
            .as_deref()
            .unwrap()
            .starts_with("The *shapes* package.")
    );
    let names: Vec<String> = pkg.members.iter().map(|m| m.qualified(".")).collect();
    assert_eq!(
        names,
        [
            "shapes.ORIGIN",
            "shapes.Point",
            "shapes.hidden",
            "shapes.Color",
            "shapes.Circle",
            "shapes.area",
            "shapes.geo",
            "shapes.stub",
            "shapes.util"
        ]
    );
    // `__all__` leaves `hidden` out.
    assert_eq!(
        find(&items, "shapes.hidden").visibility,
        Visibility::Private
    );
    assert_eq!(find(&items, "shapes.area").visibility, Visibility::Public);
    let origin = find(&items, "shapes.ORIGIN");
    assert_eq!(
        (origin.kind.clone(), origin.docs.as_deref()),
        (ItemKind::Constant, Some("Where everything starts."))
    );
    assert_eq!(signature(origin), "ORIGIN: tuple[int, int] = (0, 0)");
    assert_eq!(
        signature(find(&items, "shapes.Point")),
        "type Point = tuple[float, float]"
    );
    let color = find(&items, "shapes.Color");
    assert_eq!(color.kind, ItemKind::Enum);
    assert_eq!(
        find(&items, "shapes.Color.RED").docs.as_deref(),
        Some("Warm.")
    );
    let circle = find(&items, "shapes.Circle");
    assert_eq!(
        signature(circle),
        "class Circle(radius: float = 1.0, /, *, name: str | None = None)"
    );
    let members: Vec<(&str, Visibility)> = circle
        .members
        .iter()
        .map(|m| (m.name(), m.visibility))
        .collect();
    assert_eq!(
        members,
        [
            ("diameter", Visibility::Public),
            ("unit", Visibility::Public),
            ("grow", Visibility::Public),
            ("_private", Visibility::Private),
            ("__repr__", Visibility::Private),
            ("radius", Visibility::Public),
            ("name", Visibility::Public),
        ]
    );
    assert_eq!(
        signature(find(&items, "shapes.Circle.diameter")),
        "property diameter: float"
    );
    assert_eq!(
        signature(find(&items, "shapes.Circle.unit")),
        "classmethod unit(cls) -> \"Circle\""
    );
    assert_eq!(
        signature(find(&items, "shapes.Circle.grow")),
        "async grow[T](self, by: T, *more: T, **opts) -> None"
    );
    assert_eq!(
        signature(find(&items, "shapes.Circle.radius")),
        "radius: float = radius"
    );
    assert_eq!(
        find(&items, "shapes.Circle.name").docs.as_deref(),
        Some("Its name.")
    );
    // Stubs win over modules; the first overload stays.
    assert_eq!(
        signature(find(&items, "shapes.geo.dist")),
        "dist(a: float, b: float) -> float"
    );
    assert_eq!(signature(find(&items, "shapes.stub.f")), "f(x: int) -> int");
    assert_eq!(
        find(&items, "shapes.Circle.grow")
            .location
            .as_ref()
            .unwrap()
            .line,
        Some(48)
    );
}

#[tokio::test]
async fn documents_a_package_with_references() {
    let dir = site(
        "plugin",
        &[
            ("src/shapes/__init__.py", INIT),
            (
                "src/shapes/util.py",
                "def helper(x):\n    \"\"\"Help.\"\"\"\n",
            ),
        ],
    );
    let page = ".. py:currentmodule:: shapes\n\nSee :py:func:`area` and :py:meth:`Circle.grow`.\n\n.. py:auto:: shapes.util.helper\n";
    let plugin = plugin();
    let init = lsdocs_plugin_sdk::Setup::with_settings(
        InitializeParams {
            site: SiteInfo {
                root: Some(dir.to_string_lossy().into_owned()),
                ..SiteInfo::default()
            },
            ..InitializeParams::default()
        },
        json!({"paths": ["src/shapes"]}),
    );
    let declared = plugin.capabilities_for(&init).syntax;
    let (guide, diagnostics) = lsdocs_parser_rst::parse("guide.rst", page, None, &declared);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let ctx = context_with_parsers(
        &plugin,
        &init,
        vec![
            lsdocs_parser_rst::plugin(),
            lsdocs_parser_markdown::plugin(),
        ],
    );
    let out = run_transform_in(&plugin, &ctx, vec![guide]).await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    for (doc, diagnostics) in &out {
        assert!(diagnostics.is_empty(), "{}: {diagnostics:?}", doc.source);
    }
    let sources: Vec<&str> = out.iter().map(|(d, _)| d.source.as_str()).collect();
    assert_eq!(
        sources,
        [
            "guide.rst",
            "api/py/index.md",
            "api/py/shapes.md",
            "api/py/shapes.util.md"
        ]
    );
    let links = |nodes: &[Node]| -> Vec<String> {
        nodes
            .iter()
            .flat_map(Node::descendants)
            .filter_map(|n| match n {
                Node::Link(l) => Some(format!("{} -> {}", plain_text(&l.children), l.url)),
                _ => None,
            })
            .collect()
    };
    let (guide, _) = &out[0];
    assert_eq!(
        links(&guide.children),
        [
            "area() -> api/py/shapes.md#py.shapes.area",
            "Circle.grow() -> api/py/shapes.md#py.shapes.Circle.grow"
        ]
    );
    assert!(
        guide
            .descendants()
            .any(|n| matches!(n, Node::CodeBlock(c) if c.value == "helper(x)"))
    );
    let (shapes, _) = &out[2];
    // The package docstring is reStructuredText: emphasis, and a reference.
    assert!(
        shapes
            .descendants()
            .any(|n| matches!(n, Node::Emphasis(e) if plain_text(&e.children) == "shapes"))
    );
    let all = links(&shapes.children);
    assert!(
        all.contains(&"Circle -> #py.shapes.Circle".to_owned()),
        "{all:?}"
    );
    assert!(
        all.contains(&"area() -> #py.shapes.area".to_owned()),
        "{all:?}"
    );
    // Private members are left out.
    assert!(!shapes.descendants().any(
        |n| matches!(n, Node::Section(s) if s.id.as_deref() == Some("py.shapes.Circle._private"))
    ));
    let text = plain_text(&shapes.children);
    assert!(text.contains("If negative."), "{text}");
}

#[test]
fn hand_written_signatures() {
    let item = Python
        .describe(
            ItemKind::Function,
            "send[T](msg: T, /, *, retries=3) -> None",
            &["net".into()],
        )
        .unwrap();
    assert_eq!(item.path, ["net", "send"]);
    assert_eq!(signature(&item), "send[T](msg: T, /, *, retries=3) -> None");
    let missing = super::extract::extract(&["nope".into()], Path::new("."));
    assert_eq!(
        missing.diagnostics[0].message,
        "no such file or directory: nope"
    );
}
