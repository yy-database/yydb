// fixture: format_v0.kv_roundtrip

use yydb_format::MemoryPager;

#[test]
fn yydb_format_v0_kv_roundtrip() {
    let id = [7, 7, 7, 7, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 9];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    pager.put_kv("yydb/format", b"v1").unwrap();
    assert_eq!(pager.get_kv("yydb/format").unwrap(), Some(b"v1".to_vec()));
    assert_eq!(pager.header().unwrap().slot.record_root, 1);
}
