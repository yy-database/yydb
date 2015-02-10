# `yydb-pyo3`

Thin PyO3 binding for Python hosts. Engine logic lives in [`yydb`](https://crates.io/crates/yydb); serve loops run
through [`yydb-server`](https://crates.io/crates/yydb-server).

The binding delegates to the facade and does not import `yydb-types` directly. Python application authors use the
published wheel, not this crate.

[YYDB overview](https://github.com/yy-database/yydb)
