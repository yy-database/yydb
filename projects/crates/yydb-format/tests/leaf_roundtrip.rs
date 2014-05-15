use yydb_format::{LeafPage, TreeKey, TREE_RECORD};

#[test]
fn leaf_page_roundtrip_sorted_cells() {
    let mut leaf = LeafPage::empty(1, 1, TREE_RECORD);
    for (key, value) in [("a", b"1"), ("m", b"2"), ("z", b"3")] {
        assert!(leaf
            .upsert(TreeKey::user_record(key), value.to_vec())
            .unwrap());
    }
    let encoded = leaf.encode().unwrap();
    let decoded = LeafPage::decode(1, &encoded).unwrap();
    assert_eq!(decoded.get(&TreeKey::user_record("m")).unwrap(), b"2");
    assert_eq!(decoded.len(), 3);
}
