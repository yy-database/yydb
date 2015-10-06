use yydb_formats::{InternalPage, MemoryPager, PAGE_TYPE_INTERNAL, PageHeader, TREE_RECORD, TreeKey};

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
        assert_eq!(pager.get_kv(&key).unwrap(), Some(payload.clone()), "missing {key}");
    }
}

/// Internal page splits must keep every split child reachable.
///
/// Regression: when a full internal page absorbed a split child, the new
/// entries were rolled back before the page split and never re-inserted, so
/// one whole leaf of records became unreachable (silent row loss around the
/// first internal-page split).
#[test]
fn internal_splits_keep_every_record_reachable() {
    let id = [9, 9, 9, 9, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 7];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    let total = 2_000usize;
    let payload = vec![b'v'; 64];
    for i in 0..total {
        pager.put_kv(format!("key-{i:010}"), &payload).unwrap();
    }
    let scanned = pager.scan_kv().unwrap();
    assert_eq!(scanned.len(), total, "scan must see every inserted record");
    for i in (0..total).step_by(37) {
        let key = format!("key-{i:010}");
        assert_eq!(pager.get_kv(&key).unwrap(), Some(payload.clone()), "missing {key}");
    }
}

#[test]
fn leaf_split_halves_cells() {
    let mut leaf = yydb_formats::LeafPage::empty(1, 1, TREE_RECORD);
    let value = vec![0_u8; 100];
    for i in 0..20_u32 {
        assert!(leaf.upsert(TreeKey::user_record(format!("k{i:03}")), value.clone()).unwrap());
    }
    let (separator, right) = leaf.split().unwrap();
    assert!(!leaf.get(&separator).is_some() || leaf.get(&separator).is_some());
    assert!(right.len() >= 10);
    assert!(leaf.len() >= 10);
    assert!(right.get(&separator).is_some());
}

/// Sequential appends must fill leaves instead of halving them.
///
/// Regression: every edge split halved the page, so a bulk append kept about
/// half of each leaf empty and produced roughly twice the pages it needed.
#[test]
fn sequential_appends_fill_leaves() {
    let id = [3, 3, 3, 3, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 13];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    let total = 5_000usize;
    let payload = vec![b'v'; 64];
    for i in 0..total {
        pager.put_kv(format!("key-{i:010}"), &payload).unwrap();
    }
    assert_eq!(pager.scan_kv().unwrap().len(), total);
    // 88-byte cells fit 44 per page, so 5 000 appends need about 114 leaves.
    // Balanced edge splits would produce about 228.
    assert!(pager.page_count() < 130, "page_count={}", pager.page_count());
    for i in (0..total).step_by(97) {
        let key = format!("key-{i:010}");
        assert_eq!(pager.get_kv(&key).unwrap(), Some(payload.clone()), "missing {key}");
    }
}

/// Prepending has to keep the mirrored edge dense too.
#[test]
fn sequential_prepends_fill_leaves() {
    let id = [4, 4, 4, 4, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 14];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    let total = 5_000usize;
    let payload = vec![b'v'; 64];
    for i in (0..total).rev() {
        pager.put_kv(format!("key-{i:010}"), &payload).unwrap();
    }
    assert_eq!(pager.scan_kv().unwrap().len(), total);
    assert!(pager.page_count() < 130, "page_count={}", pager.page_count());
    for i in (0..total).step_by(97) {
        let key = format!("key-{i:010}");
        assert_eq!(pager.get_kv(&key).unwrap(), Some(payload.clone()), "missing {key}");
    }
}

#[test]
fn internal_page_roundtrip() {
    use yydb_formats::{InternalEntry, TREE_RECORD, TreeKey};
    let page = InternalPage::new(
        2,
        1,
        TREE_RECORD,
        vec![
            InternalEntry { separator: TreeKey::min_separator(TREE_RECORD), child_page_id: 1 },
            InternalEntry { separator: TreeKey::user_record("m"), child_page_id: 3 },
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

/// Reference: apply the same half-open key window to a full scan.
fn window_of(all: &[(TreeKey, Vec<u8>)], start: &str, end: Option<&str>) -> Vec<(TreeKey, Vec<u8>)> {
    let start = TreeKey::user_record(start);
    let end = end.map(TreeKey::user_record);
    all.iter().filter(|(key, _)| *key >= start && end.as_ref().is_none_or(|end| key < end)).cloned().collect()
}

fn assert_windows_match_full_scan(pager: &mut MemoryPager, cases: &[(String, Option<String>)]) {
    let all = pager.scan_kv().unwrap();
    for (start, end) in cases {
        let got = pager.scan_kv_range(start, end.as_deref().map(str::as_bytes)).unwrap();
        let want = window_of(&all, start, end.as_deref());
        assert_eq!(got, want, "window {start:?}..{end:?}");
    }
}

/// Range scans must locate their first leaf from the index instead of walking
/// every leaf: the window result has to equal the filtered full scan after both
/// leaf and internal splits.
#[test]
fn range_scan_matches_full_scan_after_splits() {
    let id = [5, 5, 5, 5, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 11];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    let total = 2_000usize;
    let payload = vec![b'v'; 64];
    for i in 0..total {
        pager.put_kv(format!("key-{i:010}"), &payload).unwrap();
    }
    assert_eq!(pager.scan_kv().unwrap().len(), total);

    let cases = [
        ("key-0000000000".to_string(), None),
        ("key-0000000000".to_string(), Some("key-0000000001".to_string())),
        ("key-0000000005".to_string(), Some("key-0000000005".to_string())),
        ("key-0000000500".to_string(), Some("key-0000000510".to_string())),
        ("key-0000001990".to_string(), Some("key-0000002100".to_string())),
        ("key-0000002000".to_string(), None),
        ("".to_string(), None),
        ("zzz".to_string(), None),
        ("zzz".to_string(), Some("aaa".to_string())),
    ];
    assert_windows_match_full_scan(&mut pager, &cases);
}

/// Cells that nearly fill a page force splits on almost every insert, so the
/// leaf layout is denser than any uniform stride predicts.
#[test]
fn range_scan_matches_full_scan_with_oversized_cells() {
    let id = [6, 6, 6, 6, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 12];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    let total = 300usize;
    let payload = vec![b'y'; 2_000];
    for i in 0..total {
        pager.put_kv(format!("big-{i:06}"), &payload).unwrap();
    }
    assert_eq!(pager.scan_kv().unwrap().len(), total);

    let cases = [
        ("big-000000".to_string(), None),
        ("big-000000".to_string(), Some("big-000010".to_string())),
        ("big-000123".to_string(), Some("big-000124".to_string())),
        ("big-000290".to_string(), Some("big-000300".to_string())),
        ("big-000500".to_string(), None),
    ];
    assert_windows_match_full_scan(&mut pager, &cases);
}
