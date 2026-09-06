//! Behavioral scenarios ported from CopyQ's test suite (src/tests/tests_items.cpp),
//! adapted where yoink's semantics intentionally differ (dedup moves to top).

use yoink_core::{HistoryStore, ItemKind};

fn texts(store: &HistoryStore, query: &str) -> Vec<String> {
    store
        .list(query, 100)
        .unwrap()
        .into_iter()
        .map(|item| item.text)
        .collect()
}

fn record_all(store: &mut HistoryStore, items: &[&str], start_ms: i64) {
    for (offset, text) in items.iter().enumerate() {
        store.record_text(text, start_ms + offset as i64).unwrap();
    }
}

// Ported from CopyQ configMaxitems.
#[test]
fn maxitems_evicts_oldest() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    store.set_max_items(3).unwrap();

    record_all(&mut store, &["A", "B", "C", "D"], 1000);
    assert_eq!(texts(&store, ""), ["D", "C", "B"]);
    assert_eq!(store.len().unwrap(), 3);

    record_all(&mut store, &["E", "F"], 2000);
    assert_eq!(texts(&store, ""), ["F", "E", "D"]);

    store.set_max_items(2).unwrap();
    assert_eq!(texts(&store, ""), ["F", "E"]);

    store.set_max_items(1).unwrap();
    assert_eq!(texts(&store, ""), ["F"]);
    store.record_text("G", 3000).unwrap();
    assert_eq!(texts(&store, ""), ["G"]);
}

// Ported from CopyQ configMaxitems: cap of zero stores nothing.
#[test]
fn maxitems_zero_stores_nothing() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    store.record_text("kept", 1000).unwrap();
    store.set_max_items(0).unwrap();
    assert!(store.is_empty().unwrap());
    assert_eq!(store.record_text("dropped", 2000).unwrap(), None);
    assert_eq!(store.record_image(b"png", b"t", 1, 1, 2000).unwrap(), None);
    assert!(store.is_empty().unwrap());
}

// Ported from CopyQ searchAccented.
#[test]
fn search_ignores_accents_and_case() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    record_all(&mut store, &["a", "väčšina", "b"], 1000);
    assert_eq!(texts(&store, "vacsina"), ["väčšina"]);
    assert_eq!(texts(&store, "VÄČŠINA"), ["väčšina"]);
    assert_eq!(texts(&store, "nothing"), Vec::<String>::new());
}

#[test]
fn newest_first_ordering() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    record_all(&mut store, &["A", "B", "C"], 1000);
    assert_eq!(texts(&store, ""), ["C", "B", "A"]);
}

// yoink-specific: re-copying existing text moves it to the top, no duplicate.
#[test]
fn dedup_moves_existing_to_top() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    record_all(&mut store, &["A", "B"], 1000);
    let item = store.record_text("A", 2000).unwrap().unwrap();
    assert_eq!(texts(&store, ""), ["A", "B"]);
    assert_eq!(store.len().unwrap(), 2);
    assert_eq!(item.created_at_ms, 1000);
    assert_eq!(item.last_copied_at_ms, 2000);
}

// CopyQ treats whitespace/null-only clipboard data as empty (hasData()).
#[test]
fn blank_text_is_ignored() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    assert_eq!(store.record_text("", 1000).unwrap(), None);
    assert_eq!(store.record_text(" \n\t\0", 1000).unwrap(), None);
    assert!(store.is_empty().unwrap());
}

#[test]
fn remove_and_clear() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    record_all(&mut store, &["A", "B"], 1000);
    let id = store.list("", 10).unwrap()[0].id;
    assert!(store.remove(id).unwrap());
    assert!(!store.remove(id).unwrap());
    assert_eq!(texts(&store, ""), ["A"]);
    store.clear().unwrap();
    assert!(store.is_empty().unwrap());
}

#[test]
fn persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");

    let mut store = HistoryStore::open(&path).unwrap();
    store.set_max_items(7).unwrap();
    record_all(&mut store, &["A", "B"], 1000);
    drop(store);

    let store = HistoryStore::open(&path).unwrap();
    assert_eq!(texts(&store, ""), ["B", "A"]);
    assert_eq!(store.max_items(), 7);
}

#[test]
fn image_dedup_moves_to_top_and_roundtrips() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    store.record_text("A", 1000).unwrap();
    let first = store
        .record_image(b"png-bytes", b"thumb-bytes", 640, 480, 1001)
        .unwrap()
        .unwrap();
    store.record_text("B", 1002).unwrap();

    // Same bytes again: moves to top, keeps identity, no duplicate.
    let again = store
        .record_image(b"png-bytes", b"thumb-bytes", 640, 480, 2000)
        .unwrap()
        .unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.created_at_ms, 1001);
    assert_eq!(again.last_copied_at_ms, 2000);
    assert_eq!(store.len().unwrap(), 3);

    let listed = store.list("", 10).unwrap();
    assert_eq!(listed[0].kind, ItemKind::Image);
    assert_eq!((listed[0].width, listed[0].height), (Some(640), Some(480)));

    assert_eq!(store.image_data(first.id).unwrap().unwrap(), b"png-bytes");
    assert_eq!(store.thumbnail(first.id).unwrap().unwrap(), b"thumb-bytes");
}

#[test]
fn text_search_excludes_images_but_empty_query_lists_all() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    store.record_text("image of a cat", 1000).unwrap();
    store.record_image(b"cat-png", b"t", 10, 10, 1001).unwrap();

    assert_eq!(store.list("", 10).unwrap().len(), 2);
    let matches = store.list("image", 10).unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].kind, ItemKind::Text);
}

#[test]
fn blob_getters_return_none_for_text_items() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    let item = store.record_text("A", 1000).unwrap().unwrap();
    assert_eq!(store.image_data(item.id).unwrap(), None);
    assert_eq!(store.thumbnail(item.id).unwrap(), None);
    assert_eq!(store.image_data(9999).unwrap(), None);
}

// A v1 (text-only) database must migrate in place with rows intact.
#[test]
fn migrates_v1_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");

    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE items (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            text TEXT NOT NULL UNIQUE,
            text_folded TEXT NOT NULL,
            created_at_ms INTEGER NOT NULL,
            last_copied_at_ms INTEGER NOT NULL
        );
        CREATE INDEX idx_items_recency ON items(last_copied_at_ms DESC, id DESC);
        CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        INSERT INTO meta (key, value) VALUES ('max_items', '42');
        INSERT INTO items (text, text_folded, created_at_ms, last_copied_at_ms)
            VALUES ('old A', 'old a', 1000, 1000), ('old B', 'old b', 1001, 1001);",
    )
    .unwrap();
    drop(conn);

    let mut store = HistoryStore::open(&path).unwrap();
    assert_eq!(store.max_items(), 42);
    assert_eq!(texts(&store, ""), ["old B", "old A"]);
    assert_eq!(texts(&store, "old a"), ["old A"]);

    // The migrated store accepts images and still dedups text.
    store.record_image(b"png", b"t", 5, 5, 2000).unwrap();
    let item = store.record_text("old A", 3000).unwrap().unwrap();
    assert_eq!(item.created_at_ms, 1000);
    assert_eq!(store.len().unwrap(), 3);

    // Reopening does not re-run the migration.
    drop(store);
    let store = HistoryStore::open(&path).unwrap();
    assert_eq!(store.len().unwrap(), 3);
}
