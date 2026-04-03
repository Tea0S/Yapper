use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    migrate(&conn)?;
    Ok(conn)
}

fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS settings (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS keybinds (
            action TEXT PRIMARY KEY NOT NULL,
            shortcut TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS dictionary (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            term TEXT NOT NULL,
            replacement TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 0,
            scope TEXT NOT NULL DEFAULT 'word'
        );

        CREATE TABLE IF NOT EXISTS corrections (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            mishear TEXT NOT NULL,
            intended TEXT NOT NULL,
            priority INTEGER NOT NULL DEFAULT 0
        );

        CREATE INDEX IF NOT EXISTS idx_dictionary_priority ON dictionary(priority DESC);
        CREATE INDEX IF NOT EXISTS idx_corrections_priority ON corrections(priority DESC);
        "#,
    )?;

    seed_defaults(conn)?;
    Ok(())
}

fn seed_defaults(conn: &Connection) -> rusqlite::Result<()> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM keybinds", [], |r| r.get(0))?;
    if count == 0 {
        let defaults = [
            ("push_to_talk", "Control+Shift+Space"),
            ("toggle_open_mic", "Control+Shift+M"),
            ("stop_dictation", "Escape"),
        ];
        for (action, shortcut) in defaults {
            conn.execute(
                "INSERT OR IGNORE INTO keybinds (action, shortcut) VALUES (?1, ?2)",
                params![action, shortcut],
            )?;
        }
    }

    let s_count: i64 = conn.query_row("SELECT COUNT(*) FROM settings", [], |r| r.get(0))?;
    if s_count == 0 {
        let defaults = [
            ("engine", "whisper"),
            ("whisper_model", "base"),
            ("compute_type", "int8"),
            ("inference_host", "local"),
            ("remote_url", "ws://127.0.0.1:8765"),
            ("remote_token", ""),
            ("tone_preset", "standard"),
            ("vad_energy_threshold", "0.008"),
            ("chunk_ms", "400"),
            ("mock_transcription", "false"),
        ];
        for (k, v) in defaults {
            conn.execute(
                "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
                params![k, v],
            )?;
        }
    }

    for (k, v) in [
        ("whisper_device", "auto"),
        ("parakeet_model", "nvidia/parakeet-tdt-0.6b-v3"),
        ("input_device_name", ""),
        ("lazy_load_whisper", "false"),
        ("model_idle_unload_mins", "0"),
        ("instance_role", "dictation"),
        ("node_server_bind", "lan"),
        ("node_server_port", "8765"),
        ("node_server_token", ""),
    ] {
        conn.execute(
            "INSERT OR IGNORE INTO settings (key, value) VALUES (?1, ?2)",
            params![k, v],
        )?;
    }
    Ok(())
}

pub fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get(0),
    )
    .optional()
}

pub fn set_setting(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeybindRow {
    pub action: String,
    pub shortcut: String,
}

pub fn list_keybinds(conn: &Connection) -> rusqlite::Result<Vec<KeybindRow>> {
    let mut stmt = conn.prepare("SELECT action, shortcut FROM keybinds ORDER BY action")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(KeybindRow {
                action: r.get(0)?,
                shortcut: r.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn set_keybind(conn: &Connection, action: &str, shortcut: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO keybinds (action, shortcut) VALUES (?1, ?2)
         ON CONFLICT(action) DO UPDATE SET shortcut = excluded.shortcut",
        params![action, shortcut],
    )?;
    Ok(())
}

pub fn check_keybind_conflicts(conn: &Connection, action: &str, shortcut: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT action FROM keybinds WHERE shortcut = ?1 AND action != ?2",
    )?;
    let conflicts: Vec<String> = stmt
        .query_map(params![shortcut, action], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(conflicts)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DictionaryEntry {
    pub id: Option<i64>,
    pub term: String,
    pub replacement: String,
    pub priority: i64,
    pub scope: String,
}

pub fn upsert_dictionary(conn: &Connection, e: &DictionaryEntry) -> rusqlite::Result<i64> {
    if let Some(id) = e.id {
        conn.execute(
            "UPDATE dictionary SET term = ?1, replacement = ?2, priority = ?3, scope = ?4 WHERE id = ?5",
            params![e.term, e.replacement, e.priority, e.scope, id],
        )?;
        Ok(id)
    } else {
        conn.execute(
            "INSERT INTO dictionary (term, replacement, priority, scope) VALUES (?1, ?2, ?3, ?4)",
            params![e.term, e.replacement, e.priority, e.scope],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

pub fn delete_dictionary(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM dictionary WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_dictionary(conn: &Connection) -> rusqlite::Result<Vec<DictionaryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, term, replacement, priority, scope FROM dictionary ORDER BY priority DESC, term",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(DictionaryEntry {
                id: Some(r.get(0)?),
                term: r.get(1)?,
                replacement: r.get(2)?,
                priority: r.get(3)?,
                scope: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn default_dict_export_priority() -> i64 {
    10
}

fn default_dict_export_scope() -> String {
    "word".into()
}

/// One row in a dictionary import/export file (no DB id — portable across devices).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DictionaryExportItem {
    pub term: String,
    pub replacement: String,
    #[serde(default = "default_dict_export_priority")]
    pub priority: i64,
    #[serde(default = "default_dict_export_scope")]
    pub scope: String,
}

fn default_dict_file_version() -> u32 {
    1
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DictionaryExportFile {
    #[serde(default)]
    pub format: String,
    #[serde(default = "default_dict_file_version")]
    pub version: u32,
    pub dictionary: Vec<DictionaryExportItem>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DictionaryImportRoot {
    File(DictionaryExportFile),
    List(Vec<DictionaryExportItem>),
}

impl DictionaryImportRoot {
    pub fn into_items(self) -> Vec<DictionaryExportItem> {
        match self {
            DictionaryImportRoot::File(f) => f.dictionary,
            DictionaryImportRoot::List(v) => v,
        }
    }
}

/// Returns `(inserted, updated)`.
pub fn import_dictionary_merge(
    conn: &Connection,
    entries: &[DictionaryExportItem],
) -> rusqlite::Result<(usize, usize)> {
    let mut inserted = 0usize;
    let mut updated = 0usize;
    let tx = conn.unchecked_transaction()?;
    for e in entries {
        let term = e.term.trim();
        if term.is_empty() {
            continue;
        }
        let replacement = e.replacement.trim();
        let rep = if replacement.is_empty() {
            term
        } else {
            replacement
        };
        let scope = if e.scope.trim().is_empty() {
            "word"
        } else {
            e.scope.trim()
        };
        let id: Option<i64> = tx
            .query_row(
                "SELECT id FROM dictionary WHERE term = ?1 AND scope = ?2",
                params![term, scope],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            tx.execute(
                "UPDATE dictionary SET replacement = ?1, priority = ?2 WHERE id = ?3",
                params![rep, e.priority, id],
            )?;
            updated += 1;
        } else {
            tx.execute(
                "INSERT INTO dictionary (term, replacement, priority, scope) VALUES (?1, ?2, ?3, ?4)",
                params![term, rep, e.priority, scope],
            )?;
            inserted += 1;
        }
    }
    tx.commit()?;
    Ok((inserted, updated))
}

pub fn import_dictionary_replace(
    conn: &Connection,
    entries: &[DictionaryExportItem],
) -> rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute("DELETE FROM dictionary", [])?;
    for e in entries {
        let term = e.term.trim();
        if term.is_empty() {
            continue;
        }
        let replacement = e.replacement.trim();
        let rep = if replacement.is_empty() {
            term
        } else {
            replacement
        };
        let scope = if e.scope.trim().is_empty() {
            "word"
        } else {
            e.scope.trim()
        };
        tx.execute(
            "INSERT INTO dictionary (term, replacement, priority, scope) VALUES (?1, ?2, ?3, ?4)",
            params![term, rep, e.priority, scope],
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CorrectionEntry {
    pub id: Option<i64>,
    pub mishear: String,
    pub intended: String,
    pub priority: i64,
}

pub fn upsert_correction(conn: &Connection, e: &CorrectionEntry) -> rusqlite::Result<i64> {
    if let Some(id) = e.id {
        conn.execute(
            "UPDATE corrections SET mishear = ?1, intended = ?2, priority = ?3 WHERE id = ?4",
            params![e.mishear, e.intended, e.priority, id],
        )?;
        Ok(id)
    } else {
        conn.execute(
            "INSERT INTO corrections (mishear, intended, priority) VALUES (?1, ?2, ?3)",
            params![e.mishear, e.intended, e.priority],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

pub fn delete_correction(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM corrections WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn list_corrections(conn: &Connection) -> rusqlite::Result<Vec<CorrectionEntry>> {
    let mut stmt = conn.prepare(
        "SELECT id, mishear, intended, priority FROM corrections ORDER BY priority DESC, LENGTH(mishear) DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(CorrectionEntry {
                id: Some(r.get(0)?),
                mishear: r.get(1)?,
                intended: r.get(2)?,
                priority: r.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn load_corrections_for_postprocess(conn: &Connection) -> rusqlite::Result<Vec<(String, String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT mishear, intended, priority FROM corrections ORDER BY priority DESC, LENGTH(mishear) DESC",
    )?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?)))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub fn load_dictionary_for_postprocess(conn: &Connection) -> rusqlite::Result<Vec<(String, String, String, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT term, replacement, scope, priority FROM dictionary ORDER BY priority DESC, LENGTH(term) DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}
