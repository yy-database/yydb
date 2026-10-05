use yydb_format::{encode_empty_page0, MemoryPager, PAGE_SIZE};

#[test]
fn memory_pager_bootstrap_page0() {
    let id = [0, 0, 0, 0, 0, 0, 0, 0, 0x40, 0x00, 0x80, 0, 0, 0, 0, 1];
    let pager = MemoryPager::new_empty(id, 0x01);
    assert_eq!(pager.page_count(), 1);
    let header = pager.header().unwrap();
    assert_eq!(header.slot.database_id, id);

    let golden = encode_empty_page0(id, 0x01);
    assert_eq!(golden.len(), PAGE_SIZE);
    let page0 = pager.get_page(0).unwrap().unwrap();
    assert_eq!(page0, golden);
}
