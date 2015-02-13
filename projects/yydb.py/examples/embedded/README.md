# Embedded example

Opens a temporary `.yydb` file, applies a small VOS schema, prints the engine
version and the `Database.info()` diagnostics, then closes the handle.

The `yydb_pyo3` extension has to be built first, for example with
`maturin build` from the package directory and installing the resulting wheel.
