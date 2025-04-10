#![allow(unused)]
mod viewer;

use clap::{Parser, Subcommand};
use colored::*;
use dirs;
use rusqlite::{params, Connection, Result as SqlResult};
use serde::{Deserialize, Serialize};
use std::{
    env,
    error::Error,
    fs,
    io::{Read, Write},
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
use tempfile::NamedTempFile;
use tokio;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gist {
    pub id: i64,
    pub content: String,
    pub tags: String,
    pub created_at: String,
}

#[derive(Parser)]
#[command(author, version, about = "A simple code snippet manager")]
#[command(long_about = "Store, search and organize your code snippets")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new snippet
    Add {
        /// Add initial tags (comma separated)
        #[arg(short, long)]
        tags: Option<String>,
        
        /// Initial content from file
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
    
    /// Update an existing snippet
    Update { 
        /// Snippet ID to update
        id: i64,
        
        /// Update tags for the snippet
        #[arg(short, long)]
        tags: Option<String>,
    },
    
    /// View snippet content
    View { 
        /// Snippet ID to view
        id: i64 
    },
    
    /// Delete a snippet
    Delete {
        /// Snippet ID to delete
        id: i64,
        
        /// Skip confirmation prompt
        #[arg(short, long)]
        force: bool,
    },
    
    /// Search for snippets
    Search { 
        /// Search query
        query: String,
        
        /// Search only in tags
        #[arg(short, long)]
        tags_only: bool,
    },
    
    /// List all snippets
    List {
        /// Limit the number of results
        #[arg(short, long, default_value = "20")]
        limit: usize,
        
        /// Sort by (created, id, tags)
        #[arg(short, long, default_value = "created")]
        sort_by: String,
    },
    
    /// Launch interactive UI
    UI,
    
    /// Export all snippets to a file
    Export {
        /// Path to export file
        #[arg(short, long)]
        output: PathBuf,
    },
    
    /// Import snippets from file
    Import {
        /// Path to import file
        #[arg(short, long)]
        input: PathBuf,
    },
    
    /// Configure application settings
    Config {
        /// Set editor command
        #[arg(long)]
        editor: Option<String>,
        
        /// Enable/disable auto tag generation
        #[arg(long)]
        auto_tags: Option<bool>,
        
        /// Set API key for tag generation
        #[arg(long)]
        api_key: Option<String>,
        
        /// Set theme (dark/light/system)
        #[arg(long)]
        theme: Option<String>,
        
        /// Show current configuration
        #[arg(short, long)]
        show: bool,
    },
    
    /// Optimize database
    Optimize,
}

#[derive(Deserialize, Serialize, Default, Clone)]
pub struct Config {
    editor: String,
    default_tags: Vec<String>,
    theme: Theme,
    auto_generate_tags: bool,
    tag_api_key: Option<String>,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum Theme {
    Dark,
    Light,
    System,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::Dark
    }
}

impl std::fmt::Display for Theme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

// Error handling and feedback
fn handle_error<T, E: std::fmt::Display>(result: Result<T, E>, operation: &str) -> Option<T> {
    match result {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("{} {}: {}", "Error".red().bold(), operation, e);
            None
        }
    }
}

fn print_success(message: &str) {
    println!("{} {}", "Success:".green().bold(), message);
}

fn prompt_confirm(message: &str) -> bool {
    print!("{} {} [y/N]: ", "Confirm:".yellow().bold(), message);
    std::io::stdout().flush().ok();
    
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    
    input.trim().to_lowercase() == "y"
}

// Configuration management
fn get_gist_dir() -> Result<PathBuf, Box<dyn Error>> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let dir = home.join(".config").join("gist");

    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn get_config_path() -> Result<PathBuf, Box<dyn Error>> {
    Ok(get_gist_dir()?.join("config.toml"))
}

fn load_config() -> Config {
    if let Ok(config_path) = get_config_path() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            if let Ok(config) = serde_json::from_str::<Config>(&content) {
                return config;
            }
        }
    }
    
    // Create default config if loading fails
    let default_config = Config {
        editor: String::new(),
        default_tags: vec!["snippet".to_string()],
        theme: Theme::Dark,
        auto_generate_tags: true,
        tag_api_key: None,
    };
    
    // Try to save default config
    if let Ok(config_path) = get_config_path() {
        if let Ok(json_str) = serde_json::to_string_pretty(&default_config) {
            let _ = fs::write(config_path, json_str);
        }
    }
    
    default_config
}

fn save_config(config: &Config) -> Result<(), Box<dyn Error>> {
    let config_path = get_config_path()?;
    let json_str = serde_json::to_string_pretty(config)?;
    fs::write(config_path, json_str)?;
    Ok(())
}

fn get_editor() -> String {
    let config = load_config();
    if !config.editor.is_empty() {
        return config.editor;
    }
    
    env::var("EDITOR").unwrap_or_else(|_| {
        if cfg!(windows) {
            "notepad".into()
        } else if Command::new("nvim").arg("--version").status().is_ok() {
            "nvim".into()
        } else if Command::new("vim").arg("--version").status().is_ok() {
            "vim".into()
        } else {
            "nano".into()
        }
    })
}

// Database functions
fn get_db_path() -> Result<PathBuf, Box<dyn Error>> {
    Ok(get_gist_dir()?.join("gists.db"))
}

fn init_db() -> Result<Connection, Box<dyn Error>> {
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

fn optimize_database(conn: &Connection) -> SqlResult<()> {
    // Run VACUUM to reclaim space and optimize the database
    conn.execute("VACUUM", [])?;
    
    // Analyze for query optimization
    conn.execute("ANALYZE", [])?;
    
    Ok(())
}

fn insert_gist(c: &Connection, content: &str, tags: &str) -> SqlResult<i64> {
    c.execute(
        "INSERT INTO gists (content, tags) VALUES (?1, ?2)",
        params![content, tags],
    )?;
    Ok(c.last_insert_rowid())
}

fn update_gist(c: &Connection, id: i64, content: &str, tags: &str) -> SqlResult<()> {
    let result = c.execute(
        "UPDATE gists SET content=?1, tags=?2 WHERE id=?3",
        params![content, tags, id],
    )?;
    
    if result == 0 {
        return Err(rusqlite::Error::QueryReturnedNoRows);
    }
    
    Ok(())
}

fn delete_gist(c: &Connection, id: i64) -> SqlResult<bool> {
    let result = c.execute("DELETE FROM gists WHERE id=?1", params![id])?;
    Ok(result > 0)
}

fn get_gist(c: &Connection, id: i64) -> SqlResult<Option<Gist>> {
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

fn search_gists(c: &Connection, query: &str, tags_only: bool) -> SqlResult<Vec<Gist>> {
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

fn list_gists(c: &Connection, limit: usize, sort_by: &str) -> SqlResult<Vec<Gist>> {
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

// Content management and editing
fn validate_content(content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("Content cannot be empty".into());
    }
    
    if content.len() > 1_000_000 {  // 1MB limit
        return Err("Content is too large (max 1MB)".into());
    }
    
    Ok(())
}

fn sanitize_tags(tags: &str) -> String {
    tags.split(',')
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .take(10)  // Limit to 10 tags
        .collect::<Vec<_>>()
        .join(", ")
}

fn edit_content(initial: Option<&str>) -> Result<String, Box<dyn Error>> {
    let mut tmp = NamedTempFile::new()?;
    if let Some(s) = initial {
        tmp.write_all(s.as_bytes())?;
    }
    let p = tmp.path();

    let editor = get_editor();
    let status = Command::new(&editor).arg(p).status()?;
    
    if !status.success() {
        return Err(format!("Editor '{}' exited with error", editor).into());
    }
    
    let mut buf = String::new();
    fs::File::open(p)?.read_to_string(&mut buf)?;
    
    if let Err(e) = validate_content(&buf) {
        return Err(e.into());
    }
    
    Ok(buf)
}

fn edit_tags(initial: Option<&str>) -> Result<String, Box<dyn Error>> {
    let default = initial.unwrap_or("").to_string();
    println!("Current tags: {}", default.cyan());
    print!("Enter new tags (comma separated, leave empty to keep current): ");
    std::io::stdout().flush()?;
    
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    
    let input = input.trim();
    if input.is_empty() {
        return Ok(default);
    }
    
    Ok(sanitize_tags(input))
}

// Tag generation with API
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

async fn get_tags(content: &str, config: &Config) -> Result<String, Box<dyn Error>> {
    // Skip if auto-generate is disabled
    if !config.auto_generate_tags {
        return Ok(config.default_tags.join(", "));
    }

    // Try using API if key is available
    if let Some(key) = &config.tag_api_key {
        let reqbody = serde_json::json!({
            "model": "openai/gpt-4o",
            "messages": [{"role":"user","content":format!("Extract 3-5 relevant tags separated by commas:\n{}",content)}],
            "temperature":0.1,
        });
        
        let client = reqwest::Client::new();
        let response = client
            .post("https://openrouter.ai/api/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", key))
            .json(&reqbody)
            .send()
            .await;
            
        // If successful, parse and return tags
        if let Ok(r) = response {
            if r.status().is_success() {
                if let Ok(resp) = r.json::<ChatResponse>().await {
                    if let Some(choice) = resp.choices.first() {
                        let tags = choice.message.content.trim().to_string();
                        return Ok(sanitize_tags(&tags));
                    }
                }
            }
        }
    }
    
    // Fallback: Extract common programming words or use default tags
    let common_langs = ["rust", "python", "javascript", "html", "css", "sql", "bash", "code", "snippet"];
    let detected: Vec<&str> = common_langs
        .iter()
        .filter(|&lang| content.to_lowercase().contains(lang))
        .copied()
        .collect();
    
    if !detected.is_empty() {
        Ok(detected.join(", "))
    } else {
        Ok(config.default_tags.join(", "))
    }
}

// Display functions
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
    let prev: String = g.content
        .lines()
        .take(3)
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(60)
        .collect();
        
    let preview = if prev.len() < g.content.len() {
        format!("{}...", prev)
    } else {
        prev
    };
    
    println!(
        "{} {} {} {} {} {}\n{}",
        "ID".bold(),
        g.id.to_string().green(),
        "| Time:".bold(),
        format_timestamp(&g.created_at),
        "| Tags:".bold(),
        g.tags.cyan(),
        preview
    );
    println!("{}", "-".repeat(60).dimmed());
}

fn format_timestamp(ts: &str) -> String {
    // Simple format for display, assuming ISO format input
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        return dt.format("%Y-%m-%d %H:%M").to_string();
    }
    ts.to_string()
}

// Import/Export functions
#[derive(Serialize, Deserialize)]
struct GistExport {
    version: u8,
    gists: Vec<Gist>,
}

fn export_gists(c: &Connection, path: &PathBuf) -> Result<usize, Box<dyn Error>> {
    let gists = list_gists(c, usize::MAX, "created_at")?;
    let export = GistExport {
        version: 1,
        gists,
    };
    
    let json = serde_json::to_string_pretty(&export)?;
    fs::write(path, json)?;
    Ok(export.gists.len())
}

fn import_gists(c: &Connection, path: &PathBuf) -> Result<usize, Box<dyn Error>> {
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

// Main function
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let conn = match init_db() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{} {}: {}", "Fatal Error".red().bold(), "Cannot initialize database", e);
            return Err(e);
        }
    };
    
    let config = load_config();

    match cli.command {
        Commands::Add { tags, file } => {
            // Get content from file or editor
            let content = if let Some(file_path) = file {
                if !file_path.exists() {
                    eprintln!("{} File not found: {:?}", "Error:".red().bold(), file_path);
                    return Ok(());
                }
                fs::read_to_string(file_path)?
            } else {
                match edit_content(None) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("{} {}", "Error editing content:".red().bold(), e);
                        return Ok(());
                    }
                }
            };
            
            if content.trim().is_empty() {
                println!("Nothing saved (empty content).");
                return Ok(());
            }
            
            // Get tags
            let tags_str = if let Some(t) = tags {
                sanitize_tags(&t)
            } else {
                match get_tags(&content, &config).await {
                    Ok(t) => t,
                    Err(_) => config.default_tags.join(", "),
                }
            };
            
            // Insert into database
            match insert_gist(&conn, &content, &tags_str) {
                Ok(id) => {
                    print_success(&format!("Saved as gist #{}", id));
                }
                Err(e) => {
                    eprintln!("{} {}", "Error saving gist:".red().bold(), e);
                }
            }
        },

        Commands::Update { id, tags } => {
            // Check if gist exists
            let gist = match get_gist(&conn, id)? {
                Some(g) => g,
                None => {
                    eprintln!("{} Gist #{} not found", "Error:".red().bold(), id);
                    return Ok(());
                }
            };
            
            // Get updated content
            let content = match edit_content(Some(&gist.content)) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{} {}", "Error editing content:".red().bold(), e);
                    return Ok(());
                }
            };
            
            if content.trim().is_empty() {
                println!("Nothing updated (empty content).");
                return Ok(());
            }
            
            // Get tags
            let tags_str = if let Some(t) = tags {
                sanitize_tags(&t)
            } else if content == gist.content {
                // If content didn't change, keep existing tags
                gist.tags
            } else {
                // Otherwise, regenerate tags
                match get_tags(&content, &config).await {
                    Ok(t) => t,
                    Err(_) => gist.tags,
                }
            };
            
            // Update in database
            match update_gist(&conn, id, &content, &tags_str) {
                Ok(_) => {
                    print_success(&format!("Updated gist #{}", id));
                }
                Err(e) => {
                    eprintln!("{} {}", "Error updating gist:".red().bold(), e);
                }
            }
        },

        Commands::View { id } => {
            match get_gist(&conn, id)? {
                Some(gist) => {
                    display_gist(&gist);
                }
                None => {
                    eprintln!("{} Gist #{} not found", "Error:".red().bold(), id);
                }
            }
        },
        
        Commands::Delete { id, force } => {
            // Check if gist exists
            match get_gist(&conn, id)? {
                Some(gist) => {
                    // Confirm deletion
                    if !force && !prompt_confirm(&format!("Delete gist #{}?", id)) {
                        println!("Deletion cancelled.");
                        return Ok(());
                    }
                    
                    // Delete from database
                    match delete_gist(&conn, id) {
                        Ok(true) => {
                            print_success(&format!("Deleted gist #{}", id));
                        }
                        Ok(false) => {
                            eprintln!("{} Gist #{} not found", "Error:".red().bold(), id);
                        }
                        Err(e) => {
                            eprintln!("{} {}", "Error deleting gist:".red().bold(), e);
                        }
                    }
                }
                None => {
                    eprintln!("{} Gist #{} not found", "Error:".red().bold(), id);
                }
            }
        },

        Commands::Search { query, tags_only } => {
            let results = search_gists(&conn, &query, tags_only)?;
            if results.is_empty() {
                println!("No results found for '{}'.", query);
                return Ok(());
            }
            
            println!("Found {} results for '{}':", results.len(), query);
            for gist in &results {
                display_gist_preview(gist);
            }
        },

        Commands::List { limit, sort_by } => {
            let results = list_gists(&conn, limit, &sort_by)?;
            if results.is_empty() {
                println!("No saved gists.");
                return Ok(());
            }
            
            println!("Showing {} gists (sorted by {}):", results.len(), sort_by);
            for gist in &results {
                display_gist_preview(gist);
            }
        },

        Commands::UI => {
            let mut all = list_gists(&conn, usize::MAX, "created_at")?;
            if all.is_empty() {
                println!("No gists found. Add some first!");
                return Ok(());
            }
            let result = viewer::run_ui(&mut all, conn, config)?;
            
            // Handle potential changes made in the UI
            match result {
                viewer::UIResult::Modified => {
                    println!("Changes saved successfully.");
                }
                viewer::UIResult::NoChanges => {
                    println!("No changes made.");
                }
                viewer::UIResult::Error(e) => {
                    eprintln!("{} {}", "Error in UI:".red().bold(), e);
                }
            }
        },
        
        Commands::Export { output } => {
            match export_gists(&conn, &output) {
                Ok(count) => {
                    print_success(&format!("Exported {} gists to {:?}", count, output));
                }
                Err(e) => {
                    eprintln!("{} {}", "Error exporting gists:".red().bold(), e);
                }
            }
        },
        
        Commands::Import { input } => {
            if !input.exists() {
                eprintln!("{} File not found: {:?}", "Error:".red().bold(), input);
                return Ok(());
            }
            
            // Confirm import
            if !prompt_confirm(&format!("Import gists from {:?}?", input)) {
                println!("Import cancelled.");
                return Ok(());
            }
            
            match import_gists(&conn, &input) {
                Ok(count) => {
                    print_success(&format!("Imported {} gists from {:?}", count, input));
                }
                Err(e) => {
                    eprintln!("{} {}", "Error importing gists:".red().bold(), e);
                }
            }
        },
        
        Commands::Config { editor, auto_tags, api_key, theme, show } => {
            let mut config = load_config();
            
            if show {
                println!("{} Configuration:", "Current".green().bold());
                println!("  {}: {}", "Editor".bold(), if config.editor.is_empty() { "(auto-detect)".dimmed().to_string() } else { config.editor.clone() });
                println!("  {}: {}", "Theme".bold(), config.theme.to_string());
                println!("  {}: {}", "Auto-generate tags".bold(), config.auto_generate_tags);
                println!("  {}: {}", "Default tags".bold(), config.default_tags.join(", "));
                println!("  {}: {}", "API Key".bold(), config.tag_api_key.map(|_| "(set)".to_string()).unwrap_or_else(|| "(not set)".dimmed().to_string()));
                return Ok(());
            }
            
            let mut changed = false;
            
            if let Some(ed) = editor {
                config.editor = ed;
                changed = true;
            }
            
            if let Some(auto) = auto_tags {
                config.auto_generate_tags = auto;
                changed = true;
            }
            
            if let Some(key) = api_key {
                config.tag_api_key = if key.is_empty() { None } else { Some(key) };
                changed = true;
            }
            
            if let Some(th) = theme {
                let new_theme = match th.to_lowercase().as_str() {
                    "dark" => Theme::Dark,
                    "light" => Theme::Light,
                    "system" => Theme::System,
                    _ => {
                        eprintln!("{} Invalid theme: {}. Use 'dark', 'light', or 'system'.", "Error:".red().bold(), th);
                        return Ok(());
                    }
                };
                config.theme = new_theme;
                changed = true;
            }
            
            if changed {
                match save_config(&config) {
                    Ok(_) => {
                        print_success("Configuration updated");
                    }
                    Err(e) => {
                        eprintln!("{} {}", "Error saving configuration:".red().bold(), e);
                    }
                }
            } else {
                println!("No configuration changes specified. Use --show to view current config.");
            }
        },
        
        Commands::Optimize => {
            println!("Optimizing database...");
            match optimize_database(&conn) {
                Ok(_) => {
                    print_success("Database optimized");
                }
                Err(e) => {
                    eprintln!("{} {}", "Error optimizing database:".red().bold(), e);
                }
            }
        },
    }

    Ok(())
}

