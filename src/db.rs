use rusqlite::{params, Connection, Result as SqlResult};
use std::{error::Error, path::PathBuf, fs};
use serde::{Deserialize, Serialize};
use crate::models::Gist;
use crate::config::get_gist_dir;

/// Get the path to the database file.
pub fn get_db_path() -> Result<PathBuf, Box<dyn Error>> {
    Ok(get_gist_dir()?.join("gists.db"))
}

/// Initialize the database connection and create tables if they don't exist.
pub fn init_db() -> Result<Connection, Box<dyn Error>> {
    let db = get_db_path()?;
    let conn = Connection::open(db)?;
    
    // Create table if it doesn't exist
    conn.execute(
        "CREATE TABLE IF NOT EXISTS gists (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            tags TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;
    
    // Create indices if they don't exist
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_gists_content ON gists(content)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_gists_tags ON gists(tags)",
        [],
    )?;
    
    Ok(conn)
}

/// Optimize the database by running VACUUM and ANALYZE.
pub fn optimize_database(conn: &Connection) -> SqlResult<()> {
    // Run VACUUM to reclaim space and optimize the database
    conn.execute("VACUUM", [])?;
    
    // Analyze for query optimization
    conn.execute("ANALYZE", [])?;
    
    Ok(())
}

/// Insert a new gist into the database.
pub fn insert_gist(c: &Connection, content: &str, tags: &str) -> SqlResult<i64> {
    c.execute(
        "INSERT INTO gists (content, tags) VALUES (?1, ?2)",
        params![content, tags],
    )?;
    Ok(c.last_insert_rowid())
}

/// Update an existing gist.
pub fn update_gist(c: &Connection, id: i64, content: &str, tags: &str) -> SqlResult<()> {
    let result = c.execute(
        "UPDATE gists SET content=?1, tags=?2 WHERE id=?3",
        params![content, tags, id],
    )?;
    
    if result == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    
    Ok(())
}

/// Delete a gist by ID.
pub fn delete_gist(c: &Connection, id: i64) -> SqlResult<bool> {
    let result = c.execute("DELETE FROM gists WHERE id=?1", params![id])?;
    Ok(result > 0)
}

/// Retrieve a gist by ID.
pub fn get_gist(c: &Connection, id: i64) -> SqlResult<Option<Gist>> {
    let result = c.query_row(
        "SELECT id, content, tags, created_at FROM gists WHERE id = ?1",
        params![id],
        |r| {
            Ok(Gist {
                id: r.get(0)?,
                content: r.get(1)?,
                tags: r.get(2)?,
                created_at: r.get(3)?,
            })
        },
    );
    
    match result {
        Ok(gist) => Ok(Some(gist)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Search gists by content or tags.
pub fn search_gists(c: &Connection, query: &str, tags_only: bool) -> SqlResult<Vec<Gist>> {
    let like = format!("%{}%", query);
    let sql = if tags_only {
        "SELECT id, content, tags, created_at FROM gists 
         WHERE tags LIKE ?1 ORDER BY created_at DESC"
    } else {
        "SELECT id, content, tags, created_at FROM gists
         WHERE content LIKE ?1 OR tags LIKE ?1 ORDER BY created_at DESC"
    };
    
    let mut stmt = c.prepare(sql)?;
    let res = stmt.query_map(params![like], |r| {
        Ok(Gist {
            id: r.get(0)?,
            content: r.get(1)?,
            tags: r.get(2)?,
            created_at: r.get(3)?,
        })
    })?;
    
    let mut out = Vec::new();
    for g in res {
        out.push(g?);
    }
    Ok(out)
}

/// List gists with sorting and limit.
pub fn list_gists(c: &Connection, limit: usize, sort_by: &str) -> SqlResult<Vec<Gist>> {
    // Validate sort_by to prevent SQL injection
    let order_by = match sort_by.to_lowercase().as_str() {
        "id" => "id",
        "tags" => "tags",
        "created" | "created_at" => "created_at",
        _ => "created_at", // Default
    };
    
    let sql = format!(
        "SELECT id, content, tags, created_at FROM gists 
         ORDER BY {} DESC LIMIT ?1", 
        order_by
    );
    
    let mut stmt = c.prepare(&sql)?;
    let res = stmt.query_map(params![limit as i64], |r| {
        Ok(Gist {
            id: r.get(0)?,
            content: r.get(1)?,
            tags: r.get(2)?,
            created_at: r.get(3)?,
        })
    })?;
    
    let mut out = Vec::new();
    for g in res {
        out.push(g?);
    }
    Ok(out)
}

#[derive(Serialize, Deserialize)]
struct GistExport {
    version: u8,
    gists: Vec<Gist>,
}

pub fn export_gists(c: &Connection, path: &PathBuf) -> Result<usize, Box<dyn Error>> {
    let gists = list_gists(c, usize::MAX, "created_at")?;
    let export = GistExport {
        version: 1,
        gists,
    };
    
    let json = serde_json::to_string_pretty(&export)?;
    fs::write(path, json)?;
    Ok(export.gists.len())
}

pub fn import_gists(c: &Connection, path: &PathBuf) -> Result<usize, Box<dyn Error>> {
    let content = fs::read_to_string(path)?;
    let import: GistExport = serde_json::from_str(&content)?;
    
    if import.gists.is_empty() {
        return Ok(0);
    }
    
    let mut count = 0;
    c.execute("BEGIN TRANSACTION", [])?;
    
    for gist in import.gists {
        let result = c.execute(
            "INSERT INTO gists (content, tags, created_at) VALUES (?1, ?2, ?3)",
            params![gist.content, gist.tags, gist.created_at],
        );
        
        if result.is_ok() {
            count += 1;
        }
    }
    
    c.execute("COMMIT", [])?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute(
            "CREATE TABLE gists (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content TEXT NOT NULL,
                tags TEXT,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            )",
            [],
        ).unwrap();
        conn
    }

    #[test]
    fn test_insert_and_get() {
        let conn = setup_db();
        let id = insert_gist(&conn, "content", "tag1, tag2").unwrap();
        let gist = get_gist(&conn, id).unwrap().unwrap();
        
        assert_eq!(gist.content, "content");
        assert_eq!(gist.tags, "tag1, tag2");
    }

    #[test]
    fn test_update() {
        let conn = setup_db();
        let id = insert_gist(&conn, "content", "tags").unwrap();
        update_gist(&conn, id, "new content", "new tags").unwrap();
        let gist = get_gist(&conn, id).unwrap().unwrap();
        
        assert_eq!(gist.content, "new content");
        assert_eq!(gist.tags, "new tags");
    }

    #[test]
    fn test_delete() {
        let conn = setup_db();
        let id = insert_gist(&conn, "content", "tags").unwrap();
        assert!(delete_gist(&conn, id).unwrap());
        assert!(get_gist(&conn, id).unwrap().is_none());
    }

    #[test]
    fn test_search() {
        let conn = setup_db();
        insert_gist(&conn, "rust code", "rust").unwrap();
        insert_gist(&conn, "python code", "python").unwrap();
        
        let results = search_gists(&conn, "rust", false).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].content, "rust code");
    }
}
