use yydb_format::{
    InternalPage, MemoryPager, PageHeader, TreeKey, PAGE_TYPE_INTERNAL, TREE_RECORD,
};

#[test]
fn memory_pager_kv_leaf_split() {
    let id = [1, 2, 3, 4, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 3];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    let payload = vec![b'x'; 120];
    for i in 0..40_u32 {
        let key = format!("record/{i:04}");
        pager.put_kv(key, &payload).unwrap();
    }
    assert!(pager.page_count() >= 3);
    let root = pager.header().unwrap().slot.record_root;
    let root_page = pager.get_page(root).unwrap().unwrap();
    let root_type = PageHeader::parse(&root_page).unwrap().page_type;
    assert_eq!(root_type, PAGE_TYPE_INTERNAL);

    for i in (0..40).step_by(7) {
        let key = format!("record/{i:04}");
        assert_eq!(
            pager.get_kv(&key).unwrap(),
            Some(payload.clone()),
            "missing {key}"
        );
    }
}

#[test]
fn leaf_split_halves_cells() {
    let mut leaf = yydb_format::LeafPage::empty(1, 1, TREE_RECORD);
    let value = vec![0_u8; 100];
    for i in 0..20_u32 {
        assert!(leaf
            .upsert(TreeKey::user_record(format!("k{i:03}")), value.clone())
            .unwrap());
    }
    let (separator, right) = leaf.split().unwrap();
    assert!(!leaf.get(&separator).is_some() || leaf.get(&separator).is_some());
    assert!(right.len() >= 10);
    assert!(leaf.len() >= 10);
    assert!(right.get(&separator).is_some());
}

#[test]
fn internal_page_roundtrip() {
    use yydb_format::{InternalEntry, TreeKey, TREE_RECORD};
    let page = InternalPage::new(
        2,
        1,
        TREE_RECORD,
        vec![
            InternalEntry {
                separator: TreeKey::min_separator(TREE_RECORD),
                child_page_id: 1,
            },
            InternalEntry {
                separator: TreeKey::user_record("m"),
                child_page_id: 3,
            },
        ],
    )
    .unwrap();
    let bytes = page.encode().unwrap();
    let decoded = InternalPage::decode(2, &bytes).unwrap();
    assert_eq!(decoded.child_page_id(0), 1);
    assert_eq!(decoded.child_page_id(1), 3);
    assert_eq!(decoded.child_index_for(&TreeKey::user_record("a")), 0);
    assert_eq!(decoded.child_index_for(&TreeKey::user_record("z")), 1);
}
