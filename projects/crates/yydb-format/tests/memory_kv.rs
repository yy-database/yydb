use yydb_format::MemoryPager;

#[test]
fn memory_pager_kv_roundtrip() {
    let id = [0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 2];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    pager.put_kv("alpha", b"one").unwrap();
    pager.put_kv("beta", b"two").unwrap();
    assert_eq!(pager.get_kv("alpha").unwrap(), Some(b"one".to_vec()));
    assert_eq!(pager.get_kv("gamma").unwrap(), None);
    pager.put_kv("alpha", b"updated").unwrap();
    assert_eq!(pager.get_kv("alpha").unwrap(), Some(b"updated".to_vec()));
    assert!(pager.delete_kv("beta").unwrap());
    assert_eq!(pager.get_kv("beta").unwrap(), None);
    assert_eq!(pager.page_count(), 2);
    let header = pager.header().unwrap();
    assert_eq!(header.slot.record_root, 1);
}
