//! Clipboard history store. No GUI or clipboard dependencies so it stays
//! testable without a display server.

mod config;

use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};

pub use config::{Config, ConfigError};

pub type Result<T> = rusqlite::Result<T>;

pub const DEFAULT_MAX_ITEMS: usize = 500;

const SCHEMA_VERSION: i64 = 2;

const ITEM_COLUMNS: &str = "id, kind, text, width, height, created_at_ms, last_copied_at_ms";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    Text,
    Image,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipItem {
    pub id: i64,
    pub kind: ItemKind,
    /// Empty for images.
    pub text: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub created_at_ms: i64,
    pub last_copied_at_ms: i64,
}

pub struct HistoryStore {
    conn: Connection,
    max_items: usize,
}

impl HistoryStore {
    pub fn open(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .mode(0o600)
                .open(path)
                .map_err(file_error)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(file_error)?;
        }
        Self::from_connection(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "secure_delete", "ON")?;
        migrate(&mut conn)?;
        let max_items = read_meta(&conn, "max_items")?
            .and_then(|value| value.parse().ok())
            .unwrap_or(DEFAULT_MAX_ITEMS);
        Ok(Self { conn, max_items })
    }

    pub fn max_items(&self) -> usize {
        self.max_items
    }

    pub fn set_max_items(&mut self, max_items: usize) -> Result<()> {
        let tx = self.conn.transaction()?;
        write_meta(&tx, "max_items", &max_items.to_string())?;
        enforce_cap(&tx, max_items)?;
        tx.commit()?;
        self.max_items = max_items;
        Ok(())
    }

    /// Stores clipboard text, or moves it to the top if already present.
    /// Returns `None` for ignored content (blank text, or a cap of zero).
    pub fn record_text(&mut self, text: &str, now_ms: i64) -> Result<Option<ClipItem>> {
        if self.max_items == 0 || is_blank(text) {
            return Ok(None);
        }
        let tx = self.conn.transaction()?;
        let recency = next_recency(&tx, now_ms)?;
        tx.execute(
            "INSERT INTO items (kind, text, text_folded, created_at_ms, last_copied_at_ms)
             VALUES ('text', ?1, ?2, ?3, ?4)
             ON CONFLICT(text) DO UPDATE SET last_copied_at_ms = ?4",
            params![text, fold(text), now_ms, recency],
        )?;
        enforce_cap(&tx, self.max_items)?;
        let item = tx.query_row(
            &format!("SELECT {ITEM_COLUMNS} FROM items WHERE text = ?1"),
            params![text],
            row_to_item,
        )?;
        tx.commit()?;
        Ok(Some(item))
    }

    /// Stores an image (already encoded, typically PNG), or moves it to the
    /// top if the same bytes are already present. Returns `None` for a cap of
    /// zero or empty data.
    pub fn record_image(
        &mut self,
        data: &[u8],
        thumb: &[u8],
        width: u32,
        height: u32,
        now_ms: i64,
    ) -> Result<Option<ClipItem>> {
        if self.max_items == 0 || data.is_empty() {
            return Ok(None);
        }
        let hash = hex_sha256(data);
        let tx = self.conn.transaction()?;
        let recency = next_recency(&tx, now_ms)?;
        tx.execute(
            "INSERT INTO items
                (kind, hash, data, thumb, width, height, created_at_ms, last_copied_at_ms)
             VALUES ('image', ?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(hash) DO UPDATE SET last_copied_at_ms = ?7",
            params![hash, data, thumb, width, height, now_ms, recency],
        )?;
        enforce_cap(&tx, self.max_items)?;
        let item = tx.query_row(
            &format!("SELECT {ITEM_COLUMNS} FROM items WHERE hash = ?1"),
            params![hash],
            row_to_item,
        )?;
        tx.commit()?;
        Ok(Some(item))
    }

    /// Newest first; `query` is an accent- and case-insensitive substring
    /// match. A non-empty query matches text items only.
    pub fn list(&self, query: &str, limit: usize) -> Result<Vec<ClipItem>> {
        let mut statement = self.conn.prepare(&format!(
            "SELECT {ITEM_COLUMNS} FROM items
             WHERE ?1 = '' OR (kind = 'text' AND instr(text_folded, ?1) > 0)
             ORDER BY last_copied_at_ms DESC, id DESC
             LIMIT ?2"
        ))?;
        let rows = statement.query_map(params![fold(query), limit as i64], row_to_item)?;
        rows.collect()
    }

    pub fn get(&self, id: i64) -> Result<Option<ClipItem>> {
        self.conn
            .query_row(
                &format!("SELECT {ITEM_COLUMNS} FROM items WHERE id = ?1"),
                params![id],
                row_to_item,
            )
            .optional()
    }

    /// Returns bounded text previews; use `get` to retrieve the full content.
    pub fn preview_page(&self, query: &str, limit: usize, offset: usize) -> Result<Vec<ClipItem>> {
        let mut statement = self.conn.prepare(
            "SELECT id, kind,
                CASE WHEN length(text) > 1000 THEN substr(text, 1, 1000) || '…' ELSE text END,
                width, height, created_at_ms, last_copied_at_ms
             FROM items
             WHERE ?1 = '' OR (kind = 'text' AND instr(text_folded, ?1) > 0)
             ORDER BY last_copied_at_ms DESC, id DESC LIMIT ?2 OFFSET ?3",
        )?;
        statement
            .query_map(
                params![
                    fold(query),
                    i64::try_from(limit).unwrap_or(i64::MAX),
                    i64::try_from(offset).unwrap_or(i64::MAX)
                ],
                row_to_item,
            )?
            .collect()
    }

    pub fn count_matches(&self, query: &str) -> Result<usize> {
        self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE ?1 = '' OR (kind = 'text' AND instr(text_folded, ?1) > 0)",
            params![fold(query)], |row| row.get::<_, i64>(0),
        ).map(|count| count as usize)
    }

    /// Full image bytes as stored; `None` for text items and unknown ids.
    pub fn image_data(&self, id: i64) -> Result<Option<Vec<u8>>> {
        self.blob_column("data", id)
    }

    /// Thumbnail bytes as stored; `None` for text items and unknown ids.
    pub fn thumbnail(&self, id: i64) -> Result<Option<Vec<u8>>> {
        self.blob_column("thumb", id)
    }

    fn blob_column(&self, column: &str, id: i64) -> Result<Option<Vec<u8>>> {
        self.conn
            .query_row(
                &format!("SELECT {column} FROM items WHERE id = ?1"),
                params![id],
                |row| row.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()
            .map(Option::flatten)
    }

    pub fn remove(&mut self, id: i64) -> Result<bool> {
        let removed = self
            .conn
            .execute("DELETE FROM items WHERE id = ?1", params![id])?;
        Ok(removed > 0)
    }

    pub fn clear(&mut self) -> Result<()> {
        self.conn.execute("DELETE FROM items", [])?;
        self.conn.execute_batch("VACUUM")?;
        Ok(())
    }

    pub fn len(&self) -> Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM items", [], |row| row.get::<_, i64>(0))
            .map(|count| count as usize)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }
}

fn enforce_cap(conn: &Connection, max_items: usize) -> Result<()> {
    conn.execute(
        "DELETE FROM items WHERE id NOT IN (
                SELECT id FROM items
                ORDER BY last_copied_at_ms DESC, id DESC
                LIMIT ?1
            )",
        params![i64::try_from(max_items).unwrap_or(i64::MAX)],
    )?;
    Ok(())
}

fn next_recency(conn: &Connection, now_ms: i64) -> Result<i64> {
    let latest: Option<i64> =
        conn.query_row("SELECT MAX(last_copied_at_ms) FROM items", [], |row| {
            row.get(0)
        })?;
    Ok(latest.map_or(now_ms, |value| now_ms.max(value.saturating_add(1))))
}

#[cfg(unix)]
fn file_error(error: std::io::Error) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
        Some(error.to_string()),
    )
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction()?;
    let recorded_version = if table_exists(&tx, "meta")? {
        read_meta(&tx, "schema_version")?
    } else {
        None
    };
    let version = match recorded_version {
        Some(value) => value.parse::<i64>().map_err(|_| schema_error(&value))?,
        None => {
            if table_exists(&tx, "items")? {
                1
            } else {
                0
            }
        }
    };
    if !(0..=SCHEMA_VERSION).contains(&version) {
        return Err(schema_error(&version.to_string()));
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;

    if version == 0 {
        tx.execute_batch(&format!("{V2_ITEMS_TABLE} {V2_ITEMS_INDEX}"))?;
    } else if version == 1 {
        // v1 was text-only with NOT NULL UNIQUE text; rebuild for image rows.
        tx.execute_batch(&format!(
            "DROP INDEX IF EXISTS idx_items_recency;
             ALTER TABLE items RENAME TO items_v1;
             {V2_ITEMS_TABLE}
             {V2_ITEMS_INDEX}
             INSERT INTO items
                 (id, kind, text, text_folded, created_at_ms, last_copied_at_ms)
                 SELECT id, 'text', text, text_folded, created_at_ms, last_copied_at_ms
                 FROM items_v1;
             DROP TABLE items_v1;"
        ))?;
    }
    if version != SCHEMA_VERSION {
        write_meta(&tx, "schema_version", &SCHEMA_VERSION.to_string())?;
    }
    tx.commit()
}

fn schema_error(version: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
        Some(format!(
            "unsupported history schema {version}; this version supports up to {SCHEMA_VERSION}"
        )),
    )
}

const V2_ITEMS_TABLE: &str = "CREATE TABLE items (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    text TEXT UNIQUE,
    text_folded TEXT,
    hash TEXT UNIQUE,
    data BLOB,
    thumb BLOB,
    width INTEGER,
    height INTEGER,
    created_at_ms INTEGER NOT NULL,
    last_copied_at_ms INTEGER NOT NULL
);";

const V2_ITEMS_INDEX: &str =
    "CREATE INDEX idx_items_recency ON items(last_copied_at_ms DESC, id DESC);";

fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
        params![name],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
}

fn read_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key],
        |row| row.get::<_, String>(0),
    )
    .optional()
}

fn write_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = ?2",
        params![key, value],
    )?;
    Ok(())
}

fn row_to_item(row: &rusqlite::Row) -> rusqlite::Result<ClipItem> {
    let kind: String = row.get(1)?;
    Ok(ClipItem {
        id: row.get(0)?,
        kind: if kind == "image" {
            ItemKind::Image
        } else {
            ItemKind::Text
        },
        text: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
        width: row.get::<_, Option<i64>>(3)?.map(|v| v as u32),
        height: row.get::<_, Option<i64>>(4)?.map(|v| v as u32),
        created_at_ms: row.get(5)?,
        last_copied_at_ms: row.get(6)?,
    })
}

fn hex_sha256(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn is_blank(text: &str) -> bool {
    text.chars().all(|c| c.is_whitespace() || c == '\0')
}

/// Lowercases and strips diacritics, so "vacsina" matches "väčšina".
fn fold(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    use unicode_normalization::char::is_combining_mark;

    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}
