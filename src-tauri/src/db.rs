use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: i64,
    pub name: String,
    pub entity_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: i64,
    pub source_id: i64,
    pub target_id: i64,
    pub relation_type: String,
    pub context: String,
    pub source_log_file: String,
}

pub fn get_db_path(app: &tauri::AppHandle) -> PathBuf {
    let path = app
        .path()
        .app_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    if !path.exists() {
        let _ = fs::create_dir_all(&path);
    }
    let db_path = path.join("graph.db");
    log::info!("Knowledge Graph Database path: {:?}", db_path);
    db_path
}

fn open_db(db_path: &Path) -> Result<Connection> {
    Connection::open(db_path)
}

pub fn init_db(app: &tauri::AppHandle) -> Result<()> {
    let path = get_db_path(app);
    let conn = open_db(&path)?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS entities (
            id INTEGER PRIMARY KEY,
            name TEXT UNIQUE COLLATE NOCASE,
            entity_type TEXT
        )",
        [],
    )?;

    // Robust Migration: Check if 'type' column exists and rename it to 'entity_type'
    let table_info: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(entities)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    if table_info.contains(&"type".to_string()) && !table_info.contains(&"entity_type".to_string()) {
        log::info!("Migrating 'entities' table: renaming 'type' to 'entity_type'");
        let _ = conn.execute("ALTER TABLE entities RENAME COLUMN type TO entity_type", []);
    }

    conn.execute(
        "CREATE TABLE IF NOT EXISTS relationships (
            id INTEGER PRIMARY KEY,
            source_id INTEGER,
            target_id INTEGER,
            relation_type TEXT,
            context TEXT,
            source_log_file TEXT,
            FOREIGN KEY(source_id) REFERENCES entities(id),
            FOREIGN KEY(target_id) REFERENCES entities(id)
        )",
        [],
    )?;

    // Indexes for fast querying
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_relationships_source ON relationships(source_id)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_relationships_target ON relationships(target_id)",
        [],
    )?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS parsed_logs (
            filename TEXT PRIMARY KEY,
            last_indexed_index INTEGER DEFAULT 0
        )",
        [],
    )?;

    // Migration for parsed_logs table: add last_indexed_index if missing
    let parsed_logs_info: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(parsed_logs)")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
        rows.filter_map(|r| r.ok()).collect()
    };

    if !parsed_logs_info.contains(&"last_indexed_index".to_string()) {
        log::info!("Migrating 'parsed_logs' table: adding 'last_indexed_index' column");
        let _ = conn.execute("ALTER TABLE parsed_logs ADD COLUMN last_indexed_index INTEGER DEFAULT 0", []);
    }

    Ok(())
}

/// Upserts an entity by name (case-insensitive due to COLLATE NOCASE in schema).
/// Returns the internal row ID.
pub fn upsert_entity(conn: &Connection, name: &str, entity_type: &str) -> Result<i64> {
    // Trim and normalize lightly
    let normalized_name = name.trim();
    if normalized_name.is_empty() {
        return Err(rusqlite::Error::QueryReturnedNoRows); // Hacky way to reject empty entities
    }

    // Try to find existing
    let mut stmt = conn.prepare("SELECT id FROM entities WHERE name = ?")?;
    let mut rows = stmt.query(params![normalized_name])?;

    if let Some(row) = rows.next()? {
        let id: i64 = row.get(0)?;
        // We could optionally update the type here, but we'll leave it as is.
        Ok(id)
    } else {
        // Insert new
        conn.execute(
            "INSERT INTO entities (name, entity_type) VALUES (?, ?)",
            params![normalized_name, entity_type],
        )?;
        Ok(conn.last_insert_rowid())
    }
}

pub fn upsert_triplets(
    app: &tauri::AppHandle,
    triplets: &serde_json::Value,
    source_log_file: &str,
) -> Result<()> {
    let path = get_db_path(app);
    let mut conn = open_db(&path)?;

    let tx = conn.transaction()?;

    if let Some(array) = triplets.as_array() {
        if array.is_empty() {
            log::debug!("No triplets found in LLM response for {}", source_log_file);
        } else {
            log::info!("Inserting {} extracted triplets from {}", array.len(), source_log_file);
        }
        for item in array {
            let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
            let relation = item.get("relation").and_then(|v| v.as_str()).unwrap_or("");
            let context = item.get("context").and_then(|v| v.as_str()).unwrap_or("");

            if source.is_empty() || target.is_empty() {
                continue;
            }

            // Upsert entities (assuming type 'Concept' if not provided explicitly by LLM)
            let source_id = match upsert_entity(&tx, source, "Concept") {
                Ok(id) => id,
                Err(_) => continue,
            };
            let target_id = match upsert_entity(&tx, target, "Concept") {
                Ok(id) => id,
                Err(_) => continue,
            };

            // Insert relationship if it doesn't already exist from the SAME log file
            // (prevents duplicates if parsing the same file twice, though migration checks first)
            let mut stmt = tx.prepare(
                "SELECT id FROM relationships 
                 WHERE source_id = ? AND target_id = ? AND relation_type = ? AND source_log_file = ?"
            )?;
            let exists = stmt.exists(params![source_id, target_id, relation, source_log_file])?;

            if !exists {
                tx.execute(
                    "INSERT INTO relationships (source_id, target_id, relation_type, context, source_log_file)
                     VALUES (?, ?, ?, ?, ?)",
                    params![source_id, target_id, relation, context, source_log_file],
                )?;
            }
        }
    }

    tx.commit()?;
    Ok(())
}

pub fn get_log_parsing_index(app: &tauri::AppHandle, filename: &str) -> Result<usize> {
    let path = get_db_path(app);
    let conn = open_db(&path)?;
    let mut stmt = conn.prepare("SELECT last_indexed_index FROM parsed_logs WHERE filename = ?")?;
    let mut rows = stmt.query(params![filename])?;
    if let Some(row) = rows.next()? {
        let val: i64 = row.get(0)?;
        Ok(val as usize)
    } else {
        Ok(0)
    }
}

pub fn update_log_parsing_index(app: &tauri::AppHandle, filename: &str, index: usize) -> Result<()> {
    let path = get_db_path(app);
    let conn = open_db(&path)?;
    conn.execute(
        "INSERT INTO parsed_logs (filename, last_indexed_index) 
         VALUES (?1, ?2) 
         ON CONFLICT(filename) DO UPDATE SET last_indexed_index = ?2",
        params![filename, index as i64],
    )?;
    Ok(())
}

pub fn has_log_file_been_parsed(app: &tauri::AppHandle, filename: &str) -> Result<bool> {
    // We consider it parsed if a record exists. 
    // But 'migration.rs' will now use 'get_log_parsing_index' to compare count.
    let path = get_db_path(app);
    let conn = open_db(&path)?;
    let mut stmt = conn.prepare("SELECT 1 FROM parsed_logs WHERE filename = ?")?;
    stmt.exists(params![filename])
}

pub fn get_triplets(
    app: &tauri::AppHandle,
    limit: usize,
    keyword: Option<&str>,
    from_date: Option<&str>,
    to_date: Option<&str>,
) -> Result<Vec<Relationship>> {
    let path = get_db_path(app);
    let conn = open_db(&path)?;
    
    let mut results = Vec::new();
    let mut sql_params: Vec<String> = Vec::new();

    let mut query = "SELECT r.id, r.source_id, r.target_id, r.relation_type, r.context, r.source_log_file 
                     FROM relationships r
                     INNER JOIN entities e1 ON r.source_id = e1.id
                     INNER JOIN entities e2 ON r.target_id = e2.id
                     WHERE 1=1".to_string();
    
    if let Some(kw) = keyword {
        query.push_str(" AND (r.context LIKE ? OR r.relation_type LIKE ? OR e1.name LIKE ? OR e2.name LIKE ?)");
        let pattern = format!("%{}%", kw);
        for _ in 0..4 {
            sql_params.push(pattern.clone());
        }
    }

    if let Some(from) = from_date {
        if !from.is_empty() {
            query.push_str(" AND r.source_log_file >= ?");
            sql_params.push(format!("{}.json", from));
        }
    }

    if let Some(to) = to_date {
        if !to.is_empty() {
            query.push_str(" AND r.source_log_file <= ?");
            sql_params.push(format!("{}.json", to));
        }
    }

    query.push_str(" ORDER BY r.id DESC");

    if limit > 0 {
        query.push_str(&format!(" LIMIT {}", limit));
    }

    let mut stmt = conn.prepare(&query)?;
    let iter = stmt.query_map(rusqlite::params_from_iter(sql_params), |row| {
        Ok(Relationship {
            id: row.get(0)?,
            source_id: row.get(1)?,
            target_id: row.get(2)?,
            relation_type: row.get(3)?,
            context: row.get(4)?,
            source_log_file: row.get(5)?,
        })
    })?;

    for item in iter {
        results.push(item?);
    }
    
    Ok(results)
}

pub fn get_all_entities(app: &tauri::AppHandle) -> Result<Vec<Entity>> {
    let path = get_db_path(app);
    let conn = open_db(&path)?;
    
    let mut stmt = conn.prepare("SELECT id, name, entity_type FROM entities")?;
    let iter = stmt.query_map([], |row| {
        Ok(Entity {
            id: row.get(0)?,
            name: row.get(1)?,
            entity_type: row.get(2)?,
        })
    })?;
    
    let mut results = Vec::new();
    for item in iter {
        results.push(item?);
    }
    Ok(results)
}
