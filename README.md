# lsdocs-apidoc-python

Part of [lsdocs](https://github.com/lsdocs), a static-site generator whose
parsers, transformers and renderers are plugins.

`crates/apidoc-python` (`lsdocs-apidoc-python`, builtin `apidoc-python`):
documents Python packages from their source with ruff's parser (no Python
runtime), in the `py` domain. Its MSRV is 1.97 (ruff's crates).

The plugin's settings and output are described in the lsdocs
documentation (https://lsdocs.github.io, "apidoc-python").

## Known limitations

- Re-exports (`from ._native import Context` in a package's `__init__`)
  document nothing; Sphinx's autodoc imports the package and would show
  them.
- **Native extension modules are not supported.** ruff's parser reads
  source; a compiled module (`.so`, `.pyd`) has none, so only a `.pyi`
  stub beside it is documented, with the stub's docstrings. Its own
  docstrings (`__doc__`, `__text_signature__`) need a Python runtime
  importing it.

## TODO

- [ ] Re-exports.
- [ ] Native extension modules: an optional runtime mode (or a plugin in
      Python) if `.pyi` stubs prove not enough.
