//! Interim VOS **subset** parser for the `yydb generate` MVP.
//!
//! Full grammar belongs in `vos-parser` (`vos` git @ `dev`). This module only
//! accepts a tiny surface used by fixtures and early typed clients:
//!
//! ```vos
//! table Setting {
//!     @@id: uuid,
//!     key: utf8,
//!     value: utf8,
//! }
//! ```
//!
//! Replace with `vos::parser` once it exposes a stable check/parse API.

use super::ir::{Field, ScalarType, Table, TsSchemaIr};
use super::Error;

pub fn parse_vos_subset(source: &str, schema_version: u32) -> Result<TsSchemaIr, Error> {
    // Keep the voml/vos-language facade linked as language authority.
    let _span = vos::ast::Span {
        start: 0,
        end: source.len(),
    };
    let _ = (_span, core::any::type_name::<vos::parser::vos_ast::Span>());

    if source.contains('\0') {
        return Err(Error::Parse(
            "VOS document must not contain NUL bytes".into(),
        ));
    }

    // Editors / Windows tools often write UTF-8 BOM; treat it as whitespace.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    // Keep embedded APP_SCHEMA / fixtures independent of checkout EOL.
    let source = source.replace("\r\n", "\n").replace('\r', "\n");

    let mut lexer = Lexer::new(&source);
    let mut tables = Vec::new();
    while !lexer.eof() {
        lexer.skip_ws_and_comments();
        if lexer.eof() {
            break;
        }
        tables.push(parse_table(&mut lexer)?);
    }
    if tables.is_empty() {
        return Err(Error::Parse(
            "expected at least one `table` declaration".into(),
        ));
    }
    Ok(TsSchemaIr {
        vos_source: source,
        schema_version,
        tables,
    })
}

fn parse_table(lexer: &mut Lexer<'_>) -> Result<Table, Error> {
    lexer.expect_ident("table")?;
    let name = lexer.expect_any_ident()?;
    lexer.expect_punct('{')?;
    let mut fields = Vec::new();
    loop {
        lexer.skip_ws_and_comments();
        if lexer.eat_punct('}') {
            break;
        }
        if lexer.eat_punct('@') {
            lexer.expect_punct('@')?;
            let attr = lexer.expect_any_ident()?;
            if attr != "id" {
                return Err(Error::Parse(format!(
                    "unsupported table attribute `@@{attr}` (MVP supports `@@id` only)"
                )));
            }
            lexer.expect_punct(':')?;
            let ty = parse_scalar(lexer)?;
            fields.push(Field {
                name: "id".into(),
                ty,
                is_id: true,
            });
        } else {
            let field_name = lexer.expect_any_ident()?;
            lexer.expect_punct(':')?;
            let ty = parse_scalar(lexer)?;
            fields.push(Field {
                name: field_name,
                ty,
                is_id: false,
            });
        }
        lexer.skip_ws_and_comments();
        let _ = lexer.eat_punct(',');
    }
    if !fields.iter().any(|f| f.is_id) {
        return Err(Error::Parse(format!(
            "table `{name}` requires `@@id: <type>`"
        )));
    }
    Ok(Table { name, fields })
}

fn parse_scalar(lexer: &mut Lexer<'_>) -> Result<ScalarType, Error> {
    let name = lexer.expect_any_ident()?;
    match name.as_str() {
        "uuid" => Ok(ScalarType::Uuid),
        "utf8" => Ok(ScalarType::Utf8),
        "bool" => Ok(ScalarType::Bool),
        "i64" => Ok(ScalarType::I64),
        "f64" => Ok(ScalarType::F64),
        other => Err(Error::Parse(format!(
            "unsupported scalar type `{other}` (MVP: uuid, utf8, bool, i64, f64)"
        ))),
    }
}

struct Lexer<'a> {
    src: &'a str,
    i: usize,
}

impl<'a> Lexer<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, i: 0 }
    }

    fn eof(&self) -> bool {
        self.i >= self.src.len()
    }

    fn peek(&self) -> Option<char> {
        self.src[self.i..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.i += ch.len_utf8();
        Some(ch)
    }

    fn skip_ws_and_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(c) if c.is_whitespace()) {
                self.bump();
            }
            if self.src[self.i..].starts_with("//") {
                while let Some(c) = self.bump() {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }
            break;
        }
    }

    fn eat_punct(&mut self, expected: char) -> bool {
        self.skip_ws_and_comments();
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, expected: char) -> Result<(), Error> {
        if self.eat_punct(expected) {
            Ok(())
        } else {
            Err(Error::Parse(format!(
                "expected `{expected}` at byte {}",
                self.i
            )))
        }
    }

    fn expect_ident(&mut self, expected: &str) -> Result<(), Error> {
        let got = self.expect_any_ident()?;
        if got == expected {
            Ok(())
        } else {
            Err(Error::Parse(format!(
                "expected `{expected}`, found `{got}`"
            )))
        }
    }

    fn expect_any_ident(&mut self) -> Result<String, Error> {
        self.skip_ws_and_comments();
        let start = self.i;
        let Some(first) = self.peek() else {
            return Err(Error::Parse("expected identifier".into()));
        };
        if !(first.is_ascii_alphabetic() || first == '_') {
            return Err(Error::Parse(format!(
                "expected identifier at byte {}",
                self.i
            )));
        }
        self.bump();
        while matches!(self.peek(), Some(c) if c.is_ascii_alphanumeric() || c == '_') {
            self.bump();
        }
        Ok(self.src[start..self.i].to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_setting_table() {
        let ir = parse_vos_subset(
            r#"
            table Setting {
                @@id: uuid,
                key: utf8,
                value: utf8,
            }
            "#,
            1,
        )
        .unwrap();
        assert_eq!(ir.tables.len(), 1);
        assert_eq!(ir.tables[0].name, "Setting");
        assert_eq!(ir.tables[0].fields.len(), 3);
        assert!(ir.tables[0].fields[0].is_id);
    }
}
