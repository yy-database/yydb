use yydb_formats::MemoryPager;

fn build(id: [u8; 16]) -> MemoryPager {
    MemoryPager::new_empty(id, 0x01)
}

fn contents(pager: &mut MemoryPager) -> Vec<(Vec<u8>, Vec<u8>)> {
    pager
        .scan_kv()
        .unwrap()
        .into_iter()
        .map(|(key, value)| (key.to_bytes().to_vec(), value))
        .collect()
}

/// A batch must merge into the owning leaves instead of splitting per row, and
/// every row must be readable afterwards.
#[test]
fn fresh_batch_packs_rows_and_keeps_them_readable() {
    let mut pager = build([2, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 1]);
    let payload = vec![b'v'; 64];
    let rows = (0..2_000u32).map(|i| (format!("key-{i:010}").into_bytes(), payload.clone())).collect();
    pager.put_kv_batch(rows).unwrap();
    let all = pager.scan_kv().unwrap();
    assert_eq!(all.len(), 2_000);
    for i in (0..2_000).step_by(37) {
        let key = format!("key-{i:010}");
        assert_eq!(pager.get_kv(&key).unwrap(), Some(payload.clone()), "missing {key}");
    }
    // Packed pages hold more than the half-full split target.
    assert!(pager.page_count() < 200, "expected dense packing, got {} pages", pager.page_count());
}

/// A batch over an existing tree must read, write and split the same rows a
/// sequence of single puts would.
#[test]
fn batch_matches_single_put_contents() {
    let payload = |i: u32| vec![(i % 251) as u8; 64];
    let existing: Vec<u32> = (0..1_000u32).map(|i| i * 3).collect();
    let mut reference = build([3, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 2]);
    for i in &existing {
        reference.put_kv(format!("key-{i:010}"), &payload(*i)).unwrap();
    }
    let mut pager = build([3, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 3]);
    for i in &existing {
        pager.put_kv(format!("key-{i:010}"), &payload(*i)).unwrap();
    }
    assert_eq!(contents(&mut pager), contents(&mut reference));

    let mut updated: Vec<u32> = (0..3_000u32).collect();
    updated.retain(|i| i % 7 != 0);
    let rows: Vec<(Vec<u8>, Vec<u8>)> =
        updated.iter().map(|i| (format!("key-{i:010}").into_bytes(), payload(*i + 1))).collect();
    pager.put_kv_batch(rows.clone()).unwrap();
    for (key, value) in &rows {
        reference.put_kv(key, value).unwrap();
    }
    assert_eq!(contents(&mut pager), contents(&mut reference));

    for i in (0..3_000u32).step_by(41) {
        let key = format!("key-{i:010}");
        let expected = reference.get_kv(&key).unwrap();
        assert_eq!(pager.get_kv(&key).unwrap(), expected, "mismatch at {key}");
    }
}

/// Duplicate keys inside one batch keep the last value, and a later batch
/// replaces an existing value.
#[test]
fn batch_duplicate_keys_keep_last_value() {
    let mut pager = build([4, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 4]);
    pager
        .put_kv_batch(vec![
            (b"alpha".to_vec(), b"first".to_vec()),
            (b"beta".to_vec(), b"middle".to_vec()),
            (b"alpha".to_vec(), b"last".to_vec()),
        ])
        .unwrap();
    assert_eq!(pager.get_kv("alpha").unwrap(), Some(b"last".to_vec()));
    assert_eq!(pager.get_kv("beta").unwrap(), Some(b"middle".to_vec()));
    pager.put_kv_batch(vec![(b"alpha".to_vec(), b"replaced".to_vec())]).unwrap();
    assert_eq!(pager.get_kv("alpha").unwrap(), Some(b"replaced".to_vec()));
    assert_eq!(pager.scan_kv().unwrap().len(), 2);
}

/// Range scans must still agree with a filtered full scan after a batch
/// rewrites and splits leaves.
#[test]
fn range_scan_agrees_after_batch_splits() {
    let mut pager = build([5, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 5]);
    let payload = vec![b'x'; 120];
    let rows: Vec<(Vec<u8>, Vec<u8>)> =
        (0..1_500u32).map(|i| (format!("rec-{i:06}").into_bytes(), payload.clone())).collect();
    pager.put_kv_batch(rows).unwrap();

    let all = pager.scan_kv().unwrap();
    let windows = [
        ("rec-000000", None),
        ("rec-000000", Some("rec-000010")),
        ("rec-000500", Some("rec-000510")),
        ("rec-001400", None),
        ("zzz", None),
    ];
    for (start, end) in windows {
        let got = pager.scan_kv_range(start, end.map(str::as_bytes)).unwrap();
        let start_key = yydb_formats::TreeKey::user_record(start);
        let end_key = end.map(yydb_formats::TreeKey::user_record);
        let want: Vec<_> = all
            .iter()
            .filter(|(key, _)| *key >= start_key && end_key.as_ref().is_none_or(|end| key < end))
            .cloned()
            .collect();
        assert_eq!(got, want, "window {start:?}..{end:?}");
    }
}
