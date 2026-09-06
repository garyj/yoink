use rusqlite::Connection;
use yoink_core::HistoryStore;

fn contains(path: &std::path::Path, marker: &str) -> bool {
    let needle = &marker.as_bytes()[..marker.len().min(64)];
    std::fs::read(path)
        .unwrap()
        .windows(needle.len())
        .any(|bytes| bytes == needle)
}

#[test]
fn removal_and_retention_erase_content_from_database_pages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let mut store = HistoryStore::open(&path).unwrap();
    store.set_max_items(1).unwrap();
    let marker = "SYNTHETIC-PRIVATE-CLIPBOARD-MARKER".repeat(100);
    let item = store.record_text(&marker, 1000).unwrap().unwrap();
    assert!(contains(&path, &marker));
    store.remove(item.id).unwrap();
    assert!(!contains(&path, &marker));
    store
        .record_image(marker.as_bytes(), marker.as_bytes(), 1, 1, 2000)
        .unwrap();
    assert!(contains(&path, &marker));
    store.record_text("replacement", 3000).unwrap();
    assert!(!contains(&path, &marker));
}

#[test]
fn clear_reclaims_content_deleted_by_older_versions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let marker = "SYNTHETIC-LEGACY-DELETED-CONTENT".repeat(100);
    let mut store = HistoryStore::open(&path).unwrap();
    store.record_text(&marker, 1000).unwrap();
    drop(store);
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA secure_delete = OFF; DELETE FROM items;")
        .unwrap();
    drop(conn);
    assert!(contains(&path, &marker));
    HistoryStore::open(&path).unwrap().clear().unwrap();
    assert!(!contains(&path, &marker));
}

#[cfg(unix)]
#[test]
fn new_and_existing_database_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    drop(HistoryStore::open(&path).unwrap());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    drop(HistoryStore::open(&path).unwrap());
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn unsupported_schema_is_rejected_without_modifying_database() {
    for version in ["3", "-1", "invalid"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let mut store = HistoryStore::open(&path).unwrap();
        store.record_text("preserved", 1).unwrap();
        drop(store);
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            [version],
        )
        .unwrap();
        drop(conn);
        let before = std::fs::read(&path).unwrap();
        assert!(HistoryStore::open(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }
}

#[test]
fn migration_failure_rolls_back_schema_and_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let conn = Connection::open(&path).unwrap();
    conn.execute_batch(
        "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         INSERT INTO meta VALUES ('schema_version', '1');
         CREATE TABLE items (id INTEGER PRIMARY KEY, text TEXT NOT NULL UNIQUE,
             text_folded TEXT NOT NULL, created_at_ms INTEGER NOT NULL,
             last_copied_at_ms INTEGER NOT NULL);
         INSERT INTO items VALUES (1, 'preserved', 'preserved', 1, 1);
         CREATE TRIGGER reject_version BEFORE UPDATE ON meta BEGIN
             SELECT RAISE(ABORT, 'synthetic migration failure'); END;",
    )
    .unwrap();
    drop(conn);
    assert!(HistoryStore::open(&path).is_err());
    let conn = Connection::open(&path).unwrap();
    assert_eq!(
        conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "1"
    );
    assert_eq!(
        conn.query_row("SELECT text FROM items", [], |row| row.get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
    assert!(conn.prepare("SELECT kind FROM items").is_err());
    assert!(conn.prepare("SELECT * FROM items_v1").is_err());
}

#[test]
fn clock_rollback_and_equal_timestamps_preserve_copy_order() {
    let mut store = HistoryStore::open_in_memory().unwrap();
    store.set_max_items(2).unwrap();
    store.record_text("A", 1000).unwrap();
    store.record_text("B", 2000).unwrap();
    store.record_text("C", 500).unwrap();
    store.record_text("B", 500).unwrap();
    let items = store.list("", 10).unwrap();
    assert_eq!(
        items
            .iter()
            .map(|item| item.text.as_str())
            .collect::<Vec<_>>(),
        ["B", "C"]
    );
    assert!(items[0].last_copied_at_ms > items[1].last_copied_at_ms);
}
