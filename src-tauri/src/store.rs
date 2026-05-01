use crate::models::{ClipboardItem, NewClipboardItem, Settings};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rusqlite::{params, Connection, OptionalExtension};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn new(app_data_dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(app_data_dir).map_err(|err| err.to_string())?;
        let db_path: PathBuf = app_data_dir.join("clipvault.sqlite3");
        let conn = Connection::open(db_path).map_err(|err| err.to_string())?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| err.to_string())?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|err| err.to_string())?;
        conn.busy_timeout(std::time::Duration::from_secs(2))
            .map_err(|err| err.to_string())?;

        conn.execute_batch(
            r#"
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS clipboard_items (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  kind TEXT NOT NULL CHECK(kind IN ('text', 'link', 'image')),
  content TEXT NOT NULL,
  url TEXT,
  domain TEXT,
  title TEXT,
  source_app TEXT,
  source_url TEXT,
  source_title TEXT,
  source_domain TEXT,
  hash TEXT NOT NULL UNIQUE,
  blob BLOB,
  thumb BLOB,
  created_at INTEGER NOT NULL,
  last_copied_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_clipboard_items_created_at ON clipboard_items(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_clipboard_items_kind ON clipboard_items(kind);
CREATE INDEX IF NOT EXISTS idx_clipboard_items_domain ON clipboard_items(domain);
CREATE INDEX IF NOT EXISTS idx_clipboard_items_source_domain ON clipboard_items(source_domain);

"#,
        )
        .map_err(|err| err.to_string())?;
        conn.execute_batch(
            r#"
DROP TABLE IF EXISTS clipboard_items_fts;
CREATE VIRTUAL TABLE clipboard_items_fts USING fts5(search_text);
"#,
        )
        .map_err(|err| err.to_string())?;
        rebuild_fts_locked(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn load_settings(&self) -> Result<Settings, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let value: Option<String> = conn
            .query_row("SELECT value FROM settings WHERE key = 'app'", [], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|err| err.to_string())?;

        let Some(value) = value else {
            let settings = Settings::default();
            drop(conn);
            self.save_settings(&settings)?;
            return Ok(settings);
        };

        let mut settings: Settings = serde_json::from_str(&value).map_err(|err| err.to_string())?;
        if settings.shortcut == "CommandOrControl+Shift+V" {
            settings.shortcut = Settings::default().shortcut;
            drop(conn);
            self.save_settings(&settings)?;
        }
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        let value = serde_json::to_string(settings).map_err(|err| err.to_string())?;
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute(
            "INSERT INTO settings(key, value) VALUES('app', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [value],
        )
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn save_item(&self, item: NewClipboardItem, settings: &Settings) -> Result<bool, String> {
        let now = unix_now();
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let hash = item.hash.clone();
        let latest_hash: Option<String> = conn
            .query_row(
                "SELECT hash FROM clipboard_items ORDER BY created_at DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| err.to_string())?;

        if latest_hash.as_deref() == Some(hash.as_str()) {
            conn.execute(
                "UPDATE clipboard_items SET last_copied_at = ?1 WHERE hash = ?2",
                params![now, hash],
            )
            .map_err(|err| err.to_string())?;
            return Ok(false);
        }

        let source_title = item.source.page_title.or(item.source.window_title);
        let source_app = item.source.app_name;
        let source_url = item.source.page_url;
        let source_domain = item.source.domain;

        conn.execute(
            r#"
INSERT INTO clipboard_items (
  kind, content, url, domain, title, source_app, source_url, source_title,
  source_domain, hash, blob, thumb, created_at, last_copied_at
) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?13)
ON CONFLICT(hash) DO UPDATE SET
  kind = excluded.kind,
  content = excluded.content,
  url = excluded.url,
  domain = excluded.domain,
  title = excluded.title,
  source_app = excluded.source_app,
  source_url = excluded.source_url,
  source_title = excluded.source_title,
  source_domain = excluded.source_domain,
  blob = excluded.blob,
  thumb = excluded.thumb,
  created_at = excluded.created_at,
  last_copied_at = excluded.last_copied_at
"#,
            params![
                item.kind,
                item.content,
                item.url,
                item.domain,
                item.title,
                source_app,
                source_url,
                source_title,
                source_domain,
                &hash,
                item.blob,
                item.thumb,
                now
            ],
        )
        .map_err(|err| err.to_string())?;

        let id: i64 = conn
            .query_row(
                "SELECT id FROM clipboard_items WHERE hash = ?1",
                [&hash],
                |row| row.get(0),
            )
            .map_err(|err| err.to_string())?;

        let search_text = build_search_text(id, &conn)?;
        conn.execute("DELETE FROM clipboard_items_fts WHERE rowid = ?1", [id])
            .map_err(|err| err.to_string())?;
        conn.execute(
            "INSERT INTO clipboard_items_fts(rowid, search_text) VALUES(?1, ?2)",
            params![id, search_text],
        )
        .map_err(|err| err.to_string())?;

        cleanup_locked(&conn, settings)?;
        Ok(true)
    }

    pub fn search_items(&self, query: &str, limit: i64) -> Result<Vec<ClipboardItem>, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        let query = query.trim();
        if query.is_empty() {
            return load_items(
                &conn,
                "SELECT * FROM clipboard_items ORDER BY created_at DESC, id DESC LIMIT ?1",
                params![limit],
            );
        }

        let fts_query = build_fts_query(query);
        let like = format!("%{}%", query.to_lowercase());

        if fts_query.is_empty() {
            load_items(
                &conn,
                r#"
SELECT * FROM clipboard_items
WHERE lower(content) LIKE ?1
   OR lower(coalesce(url, '')) LIKE ?1
   OR lower(coalesce(domain, '')) LIKE ?1
   OR lower(coalesce(source_domain, '')) LIKE ?1
ORDER BY created_at DESC, id DESC
LIMIT ?2
"#,
                params![like, limit],
            )
        } else {
            load_items(
                &conn,
                r#"
SELECT DISTINCT clipboard_items.*
FROM clipboard_items
LEFT JOIN clipboard_items_fts ON clipboard_items_fts.rowid = clipboard_items.id
WHERE clipboard_items.id IN (
    SELECT rowid FROM clipboard_items_fts WHERE clipboard_items_fts MATCH ?1
)
   OR lower(clipboard_items.content) LIKE ?2
   OR lower(coalesce(clipboard_items.url, '')) LIKE ?2
   OR lower(coalesce(clipboard_items.domain, '')) LIKE ?2
   OR lower(coalesce(clipboard_items.source_domain, '')) LIKE ?2
ORDER BY clipboard_items.created_at DESC, clipboard_items.id DESC
LIMIT ?3
"#,
                params![fts_query, like, limit],
            )
        }
    }

    pub fn get_item_payload(
        &self,
        id: i64,
    ) -> Result<(String, String, Option<Vec<u8>>, String), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "SELECT kind, content, blob, hash FROM clipboard_items WHERE id = ?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|err| err.to_string())
    }

    pub fn get_source_url(&self, id: i64) -> Result<String, String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.query_row(
            "SELECT source_url FROM clipboard_items WHERE id = ?1 AND source_url IS NOT NULL AND source_url != ''",
            [id],
            |row| row.get(0),
        )
        .map_err(|_| "Ten wpis nie ma zapisanego linku źródłowego".to_string())
    }

    pub fn delete_item(&self, id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute("DELETE FROM clipboard_items WHERE id = ?1", [id])
            .map_err(|err| err.to_string())?;
        conn.execute("DELETE FROM clipboard_items_fts WHERE rowid = ?1", [id])
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn clear_history(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        conn.execute("DELETE FROM clipboard_items", [])
            .map_err(|err| err.to_string())?;
        conn.execute("DELETE FROM clipboard_items_fts", [])
            .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub fn apply_cleanup(&self, settings: &Settings) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|err| err.to_string())?;
        cleanup_locked(&conn, settings)
    }
}

fn load_items<P>(conn: &Connection, sql: &str, params: P) -> Result<Vec<ClipboardItem>, String>
where
    P: rusqlite::Params,
{
    let mut statement = conn.prepare(sql).map_err(|err| err.to_string())?;
    let rows = statement
        .query_map(params, |row| {
            let thumb: Option<Vec<u8>> = row.get("thumb")?;
            Ok(ClipboardItem {
                id: row.get("id")?,
                kind: row.get("kind")?,
                content: row.get("content")?,
                url: row.get("url")?,
                domain: row.get("domain")?,
                title: row.get("title")?,
                source_app: row.get("source_app")?,
                source_url: row.get("source_url")?,
                source_title: row.get("source_title")?,
                source_domain: row.get("source_domain")?,
                created_at: row.get("created_at")?,
                thumb_data_url: thumb
                    .map(|bytes| format!("data:image/png;base64,{}", STANDARD.encode(bytes))),
            })
        })
        .map_err(|err| err.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())
}

fn build_search_text(id: i64, conn: &Connection) -> Result<String, String> {
    conn.query_row(
        r#"
SELECT printf('%s %s %s %s %s %s %s',
  content,
  coalesce(url, ''),
  coalesce(domain, ''),
  coalesce(title, ''),
  coalesce(source_url, ''),
  coalesce(source_title, ''),
  coalesce(source_domain, '')
) FROM clipboard_items WHERE id = ?1
"#,
        [id],
        |row| row.get(0),
    )
    .map_err(|err| err.to_string())
}

fn rebuild_fts_locked(conn: &Connection) -> Result<(), String> {
    let mut statement = conn
        .prepare("SELECT id FROM clipboard_items ORDER BY id")
        .map_err(|err| err.to_string())?;
    let ids = statement
        .query_map([], |row| row.get::<_, i64>(0))
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;

    for id in ids {
        let search_text = build_search_text(id, conn)?;
        conn.execute(
            "INSERT INTO clipboard_items_fts(rowid, search_text) VALUES(?1, ?2)",
            params![id, search_text],
        )
        .map_err(|err| err.to_string())?;
    }

    Ok(())
}

fn build_fts_query(query: &str) -> String {
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(|term| format!("{}*", term.to_lowercase()))
        .collect();
    terms.join(" AND ")
}

fn cleanup_locked(conn: &Connection, settings: &Settings) -> Result<(), String> {
    if let Some(days) = settings.retention_days {
        let cutoff = unix_now().saturating_sub(days.max(1) * 86_400);
        conn.execute(
            "DELETE FROM clipboard_items WHERE created_at < ?1",
            [cutoff],
        )
        .map_err(|err| err.to_string())?;
    }

    if settings.max_items > 0 {
        conn.execute(
            r#"
DELETE FROM clipboard_items
WHERE id NOT IN (
  SELECT id FROM clipboard_items ORDER BY created_at DESC, id DESC LIMIT ?1
)
"#,
            [settings.max_items],
        )
        .map_err(|err| err.to_string())?;
    }

    conn.execute(
        "DELETE FROM clipboard_items_fts WHERE rowid NOT IN (SELECT id FROM clipboard_items)",
        [],
    )
    .map_err(|err| err.to_string())?;

    Ok(())
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}
