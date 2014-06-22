use yydb::schema::{document_version, initial_version, validate_document};

#[test]
fn reads_optional_schema_version_pragma() {
    let document = "// @yydb-schema-version: 3\n\ntable User { @@id: uuid }";
    assert_eq!(document_version(document), Some(3));
    assert_eq!(initial_version(document), 3);
}

#[test]
fn defaults_initial_version_to_one() {
    assert_eq!(initial_version("table User { @@id: uuid }"), 1);
}

#[test]
fn validates_schema_through_oak_projection() {
    assert!(validate_document("table User { @@id: uuid }").is_ok());
    assert!(validate_document("table User { id: uuid }").is_err());
    assert!(validate_document("table User {").is_err());
}
