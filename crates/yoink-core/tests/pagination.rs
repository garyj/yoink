use yoink_core::HistoryStore;

#[test]
fn every_retained_image_can_be_browsed_in_pages() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    for i in 0_u32..501 {
        store
            .record_image(&i.to_le_bytes(), b"thumb", 1, 1, i64::from(i))
            .unwrap();
    }
    assert_eq!(store.count_matches("").unwrap(), 500);
    let expected = store.list("", 500).unwrap();
    let mut actual = Vec::new();
    for offset in (0..500).step_by(50) {
        actual.extend(store.preview_page("", 50, offset).unwrap());
    }
    assert_eq!(actual, expected);
    assert!(store.preview_page("", 50, 500).unwrap().is_empty());
    assert_eq!(store.count_matches("image").unwrap(), 0);
}

#[test]
fn search_uses_full_text_while_previews_remain_bounded() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    let text = format!("{} CAFÉ at the end", "🙂".repeat(3000));
    let item = store.record_text(&text, 1).unwrap().unwrap();
    let preview = store.preview_page("cafe", 50, 0).unwrap();
    assert_eq!(store.count_matches("CAFÉ").unwrap(), 1);
    assert_eq!(preview.len(), 1);
    assert_eq!(preview[0].text.chars().count(), 1001);
    assert!(preview[0].text.ends_with('…'));
    assert_eq!(store.get(item.id).unwrap().unwrap().text, text);
    assert!(store.preview_page("cafe", 50, 1).unwrap().is_empty());
}
