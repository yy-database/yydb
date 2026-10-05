// gate: format-v0 (Living 09)
// fixture: format_v0.kv_leaf_split

use yydb_format::{MemoryPager, PageHeader, PAGE_TYPE_INTERNAL};

#[test]
fn g_yydb_format_v0_kv_leaf_split() {
    let id = [9, 9, 9, 9, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 1];
    let mut pager = MemoryPager::new_empty(id, 0x01);
    let payload = vec![b'v'; 100];
    for i in 0..35_u32 {
        pager.put_kv(format!("k/{i:03}"), &payload).unwrap();
    }
    let root = pager.header().unwrap().slot.record_root;
    let root_type = PageHeader::parse(&pager.get_page(root).unwrap().unwrap())
        .unwrap()
        .page_type;
    assert_eq!(root_type, PAGE_TYPE_INTERNAL);
    assert_eq!(pager.get_kv("k/017").unwrap(), Some(payload));
}
