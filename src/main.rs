#![allow(unused)]
mod viewer;

use clap::{Parser, Subcommand};
use colored::*;
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::{
    env,
    error::Error,
    fs,
    io::{Read, Write},
    process::Command,
};
use tempfile::NamedTempFile;
use tokio;

#[derive(Debug, Clone)]
pub struct Gist {
    pub id: i64,
    pub content: String,
    pub tags: String,
    pub created_at: String,
}

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Add,
    Update { id: i64 },
    Search { query: String },
    View { id: i64 },
    List,
    UI,
}

fn get_gist_dir() -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    // ALWAYS base dir on current user's home, resolved dynamically and portable
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let dir = home.join(".config").join("gist");

    std::fs::create_dir_all(&dir)?; // Create if missing
    Ok(dir)
}

fn init_db() -> Result<Connection, Box<dyn Error>> {
    let db = get_gist_dir()?.join("gists.db");
    let conn = Connection::open(db)?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS gists (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            content TEXT NOT NULL,
            tags TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        )",
        [],
    )?;
    Ok(conn)
}

fn insert_gist(c: &Connection, content: &str, tags: &str) -> SqlResult<i64> {
    c.execute(
        "INSERT INTO gists (content, tags) VALUES (?1, ?2)",
        params![content, tags],
    )?;
    Ok(c.last_insert_rowid())
}

fn update_gist(c: &Connection, id: i64, content: &str, tags: &str) -> SqlResult<()> {
    c.execute(
        "UPDATE gists SET content=?1, tags=?2 WHERE id=?3",
        params![content, tags, id],
    )?;
    Ok(())
}

fn get_gist(c: &Connection, id: i64) -> SqlResult<Gist> {
    c.query_row(
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
    )
}

fn search_gists(c: &Connection, query: &str) -> SqlResult<Vec<Gist>> {
    let like = format!("%{}%", query);
    let mut stmt = c.prepare(
        "SELECT id, content, tags, created_at FROM gists
         WHERE content LIKE ?1 OR tags LIKE ?1 ORDER BY created_at DESC",
    )?;
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

fn list_gists(c: &Connection) -> SqlResult<Vec<Gist>> {
    let mut stmt =
        c.prepare("SELECT id, content, tags, created_at FROM gists ORDER BY created_at DESC")?;
    let res = stmt.query_map([], |r| {
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

fn edit_content(initial: Option<&str>) -> Result<String, Box<dyn Error>> {
    let mut tmp = NamedTempFile::new()?;
    if let Some(s) = initial {
        tmp.write_all(s.as_bytes())?;
    }
    let p = tmp.path();

    let ed = env::var("EDITOR").unwrap_or("nvim".into());
    let status = Command::new(ed).arg(p).status()?;
    if !status.success() {
        return Err("Editor exited with error".into());
    }
    let mut buf = String::new();
    fs::File::open(p)?.read_to_string(&mut buf)?;
    Ok(buf)
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
}
#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}
#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}
#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}
#[derive(Deserialize)]
struct ChatMessageResponse {
    role: String,
    content: String,
}

async fn get_tags(content: &str) -> Result<String, Box<dyn Error>> {
    let key = env::var("OPENROUTER_API_KEY")?;
    let reqbody = serde_json::json!({
        "model": "openai/gpt-4o",
        "messages": [{"role":"user","content":format!("Extract 3-5 relevant tags separated by commas:\n{}",content)}],
        "temperature":0.1,
    });
    let c = reqwest::Client::new();
    let r = c
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", key))
        .json(&reqbody)
        .send()
        .await?;
    if !r.status().is_success() {
        return Err(format!("API error: {}", r.text().await?).into());
    }
    let resp: ChatResponse = r.json().await?;
    Ok(resp
        .choices
        .first()
        .map(|c| c.message.content.trim().to_string())
        .unwrap_or_default())
}

fn display_gist(g: &Gist) {
    println!(
        "{} {}\n{} {}\n{} {}\n\n{}",
        "ID:".bold(),
        g.id.to_string().green(),
        "Created:".bold(),
        g.created_at,
        "Tags:".bold(),
        g.tags.cyan(),
        g.content
    );
    println!("{}", "-".repeat(50).dimmed());
}

fn display_gist_preview(g: &Gist) {
    let prev = g.content.lines().take(3).collect::<Vec<_>>().join(" ");
    println!(
        "{} {} {} {} {} {}\n{}",
        "ID".bold(),
        g.id.to_string().green(),
        "| Time:".bold(),
        g.created_at,
        "| Tags:".bold(),
        g.tags.cyan(),
        prev
    );
    println!("{}", "-".repeat(40).dimmed());
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let conn = init_db()?;

    match cli.command {
        Commands::Add => {
            let content = edit_content(None)?;
            if content.trim().is_empty() {
                println!("Nothing saved.");
                return Ok(());
            }
            let tags = get_tags(&content).await.unwrap_or("untagged".into());
            let id = insert_gist(&conn, &content, &tags)?;
            println!("Saved as {}", id);
        }

        Commands::Update { id } => {
            let old = get_gist(&conn, id)?;
            let content = edit_content(Some(&old.content))?;
            if content.trim().is_empty() {
                println!("Nothing updated");
                return Ok(());
            }
            let tags = get_tags(&content).await.unwrap_or(old.tags);
            update_gist(&conn, id, &content, &tags)?;
            println!("Updated");
        }

        Commands::View { id } => {
            display_gist(&get_gist(&conn, id)?);
        }

        Commands::Search { query } => {
            let v = search_gists(&conn, &query)?;
            if v.is_empty() {
                println!("No results.");
            }
            for g in &v {
                display_gist_preview(g);
            }
        }

        Commands::List => {
            let v = list_gists(&conn)?;
            if v.is_empty() {
                println!("No saved gists.");
            }
            for g in &v {
                display_gist_preview(g);
            }
        }

        Commands::UI => {
            let all = list_gists(&conn)?;
            if all.is_empty() {
                println!("No gists found. Add some first!");
                return Ok(());
            }
            viewer::run_ui(&all)?;
        }
    }
    Ok(())
}
