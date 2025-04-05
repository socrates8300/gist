#![allow(unused)]
use clap::{Parser, Subcommand};
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::env;
use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::NamedTempFile;
use tokio;

//
// Data structures for the SQLite "gist" record
//
struct Gist {
    id: i64,
    content: String,
    tags: String,
    created_at: String,
}

//
// CLI definitions using clap
//
#[derive(Parser)]
#[command(
    author = "Your Name <you@example.com>",
    version = "0.1.0",
    about = "A simple CLI program to store, search, update, and retrieve gists with AI auto-tagging."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new gist entry. Will spawn nvim to edit.
    Add,
    /// Update an existing gist entry by id.
    Update {
        /// The id of the gist to update.
        id: i64,
    },
    /// Search gists by query (matches both content and tags).
    /// If a number is provided, it will be interpreted as an ID lookup.
    Search {
        /// The query string to search for, or gist ID if numeric.
        query: String,
    },
    /// View a gist by id.
    View {
        /// The id of the gist to view.
        id: i64,
    },
    /// List all gists.
    List,
}

//
// Structures and types for calling OpenRouter API. Assumes a ChatCompletion-style endpoint.
//
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

/// Get the path to the gist directory
///
/// Will use the following locations in order of precedence:
/// 1. GIST_DIR environment variable
/// 2. ~/.config/gist/
fn get_gist_dir() -> Result<PathBuf, Box<dyn Error>> {
    // Try to get the directory from the environment variable
    if let Ok(dir) = env::var("GIST_DIR") {
        let path = PathBuf::from(dir);
        fs::create_dir_all(&path)?;
        return Ok(path);
    }

    // Otherwise use ~/.config/gist/
    if let Some(home_dir) = home::home_dir() {
        let path = home_dir.join(".config").join("gist");
        fs::create_dir_all(&path)?;
        return Ok(path);
    }

    Err("Could not determine gist directory. Please set GIST_DIR environment variable.".into())
}

//
// Opens (or creates) the SQLite database and ensures the table exists.
//
fn init_db() -> Result<Connection, Box<dyn Error>> {
    let db_path = get_gist_dir()?;

    let db_file = db_path.join("gists.db");
    let conn = Connection::open(db_file)?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS gists (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            content    TEXT NOT NULL,
            tags       TEXT,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
         )",
        [],
    )?;
    Ok(conn)
}

//
// Launches Neovim on a temporary file. If an initial content is provided then writes it first.
// Returns the content the user enters in the editor.
//
fn edit_content(initial: Option<&str>) -> Result<String, Box<dyn Error>> {
    let mut tmpfile = NamedTempFile::new()?;
    if let Some(text) = initial {
        tmpfile.write_all(text.as_bytes())?;
    }
    let path = tmpfile.path().to_owned();

    let status = Command::new("nvim").arg(&path).status()?;
    if !status.success() {
        return Err("nvim did not exit successfully".into());
    }

    let content = fs::read_to_string(&path)?;
    Ok(content)
}

//
// Inserts a new gist into the database and returns the inserted row id.
//
fn insert_gist(conn: &Connection, content: &str, tags: &str) -> SqlResult<i64> {
    conn.execute(
        "INSERT INTO gists (content, tags) VALUES (?1, ?2)",
        params![content, tags],
    )?;
    Ok(conn.last_insert_rowid())
}

//
// Updates an existing gist in the database.
fn update_gist(conn: &Connection, id: i64, content: &str, tags: &str) -> SqlResult<()> {
    conn.execute(
        "UPDATE gists SET content = ?1, tags = ?2 WHERE id = ?3",
        params![content, tags, id],
    )?;
    Ok(())
}

//
// Retrieves a gist by id.
fn get_gist(conn: &Connection, id: i64) -> SqlResult<Gist> {
    conn.query_row(
        "SELECT id, content, tags, created_at FROM gists WHERE id = ?1",
        params![id],
        |row| {
            Ok(Gist {
                id: row.get(0)?,
                content: row.get(1)?,
                tags: row.get(2)?,
                created_at: row.get(3)?,
            })
        },
    )
}

//
// Searches for gists that match the query in either the tags or content fields.
fn search_gists(conn: &Connection, query: &str) -> SqlResult<Vec<Gist>> {
    let like_query = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT id, content, tags, created_at FROM gists
         WHERE content LIKE ?1 OR tags LIKE ?1
         ORDER BY created_at DESC",
    )?;

    let gist_iter = stmt.query_map(params![like_query], |row| {
        Ok(Gist {
            id: row.get(0)?,
            content: row.get(1)?,
            tags: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;

    let mut gists = Vec::new();
    for gist in gist_iter {
        gists.push(gist?);
    }
    Ok(gists)
}

//
// Returns a list of all gists in the database.
fn list_gists(conn: &Connection) -> SqlResult<Vec<Gist>> {
    let mut stmt =
        conn.prepare("SELECT id, content, tags, created_at FROM gists ORDER BY created_at DESC")?;

    let gist_iter = stmt.query_map([], |row| {
        Ok(Gist {
            id: row.get(0)?,
            content: row.get(1)?,
            tags: row.get(2)?,
            created_at: row.get(3)?,
        })
    })?;

    let mut gists = Vec::new();
    for gist in gist_iter {
        gists.push(gist?);
    }
    Ok(gists)
}

//
// Uses OpenRouter API to auto-tag gist content.
//
async fn get_tags(content: &str) -> Result<String, Box<dyn Error>> {
    let api_key = env::var("OPENROUTER_API_KEY")
        .map_err(|_| "OPENROUTER_API_KEY environment variable not set")?;

    // Create the request payload using serde_json for flexibility
    let request_body = serde_json::json!({
        "model": "openai/gpt-4o",
        "messages": [
            {
                "role": "user",
                "content": format!("Extract 3-5 relevant tags or keywords from this text, separated by commas: {}", content)
            }
        ],
        "temperature": 0.1
    });

    // Set up a client with timeout for better error handling
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // Make the API request with proper error handling
    let response = match client
        .post("https://openrouter.ai/api/v1/chat/completions")
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&request_body)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("Network error details: {:?}", e);
            return Err(format!("error sending request for url: {}", e).into());
        }
    };

    // Get the status before consuming the response
    let status = response.status();

    if !status.is_success() {
        let error_text = match response.text().await {
            Ok(text) => text,
            Err(_) => "Could not read error response".to_string(),
        };
        return Err(format!(
            "Failed to call OpenRouter API: status code {}. Details: {}",
            status, error_text
        )
        .into());
    }

    // Parse the JSON response with error handling
    match response.json::<ChatResponse>().await {
        Ok(json) => {
            if let Some(choice) = json.choices.first() {
                Ok(choice.message.content.trim().to_string())
            } else {
                Err("No choices in OpenRouter API response".into())
            }
        }
        Err(e) => Err(format!("Failed to parse JSON response: {}", e).into()),
    }
}

//
// The main function -- dispatches commands
//
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    // Initialize the database with the appropriate path
    let conn = init_db()?;

    // Get the directory we're using for reference
    let data_dir = get_gist_dir()?;
    println!("Using database at: {}", data_dir.join("gists.db").display());

    match cli.command {
        Commands::Add => {
            println!("Opening nvim to create a new gist. Save and exit to continue.");
            let content = edit_content(None)?;
            if content.trim().is_empty() {
                println!("Aborting: gist content is empty.");
                return Ok(());
            }

            let tags = match get_tags(&content).await {
                Ok(tags) => tags,
                Err(e) => {
                    eprintln!("Warning: could not generate tags: {}. Using 'untagged'.", e);
                    "untagged".to_string()
                }
            };

            let id = insert_gist(&conn, &content, &tags)?;
            println!("Gist added with id: {}", id);
        }
        Commands::Update { id } => {
            let gist = get_gist(&conn, id)?;
            println!("Opening nvim to update gist id: {}", id);
            let updated_content = edit_content(Some(&gist.content))?;
            if updated_content.trim().is_empty() {
                println!("Aborting update: gist content is empty.");
                return Ok(());
            }

            let updated_tags = match get_tags(&updated_content).await {
                Ok(tags) => tags,
                Err(e) => {
                    eprintln!(
                        "Warning: could not generate tags: {}. Keeping previous tags.",
                        e
                    );
                    gist.tags.clone()
                }
            };

            update_gist(&conn, id, &updated_content, &updated_tags)?;
            println!("Gist updated.");
        }
        Commands::Search { query } => {
            // Check if query is a valid number (gist ID)
            if let Ok(id) = query.parse::<i64>() {
                // If it's a number, do an ID lookup instead
                match get_gist(&conn, id) {
                    Ok(gist) => {
                        println!("ID: {}", gist.id);
                        println!("Created At: {}", gist.created_at);
                        println!("Tags: {}", gist.tags);
                        println!("Content:\n{}", gist.content);
                    }
                    Err(_) => {
                        println!("No gist found with ID: {}", id);
                    }
                }
            } else {
                // Regular search by content/tags
                let results = search_gists(&conn, &query)?;
                if results.is_empty() {
                    println!("No gists found matching query: '{}'", query);
                } else {
                    println!("Found {} gists:", results.len());
                    for gist in results {
                        println!(
                            "ID: {} | Date: {} | Tags: {}\nPreview: {}\n",
                            gist.id,
                            gist.created_at,
                            gist.tags,
                            gist.content.lines().take(3).collect::<Vec<_>>().join(" ")
                        );
                    }
                }
            }
        }
        Commands::View { id } => {
            let gist = get_gist(&conn, id)?;
            println!("ID: {}", gist.id);
            println!("Created At: {}", gist.created_at);
            println!("Tags: {}", gist.tags);
            println!("Content:\n{}", gist.content);
        }
        Commands::List => {
            let results = list_gists(&conn)?;
            if results.is_empty() {
                println!("No gists have been added yet.");
            } else {
                println!("Listing {} gists:", results.len());
                for gist in results {
                    println!(
                        "ID: {} | Date: {} | Tags: {}\nPreview: {}\n",
                        gist.id,
                        gist.created_at,
                        gist.tags,
                        gist.content.lines().take(3).collect::<Vec<_>>().join(" ")
                    );
                }
            }
        }
    }
    Ok(())
}
