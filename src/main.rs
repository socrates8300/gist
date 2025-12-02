#![allow(unused)]
mod viewer;
mod models;
mod config;
mod db;
mod ai;
mod utils;

use clap::{Parser, Subcommand};
use colored::*;
use std::{error::Error, path::PathBuf, io::Write};
use crate::models::{Gist, Theme};
use crate::config::{load_config, save_config, Config};
use crate::db::*;
use crate::ai::get_tags;
use crate::utils::{edit_content, prompt_confirm, validate_content};

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

fn print_success(message: &str) {
    println!("{} {}", "Success:".green().bold(), message);
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
                std::fs::read_to_string(file_path)?
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
                crate::ai::sanitize_tags(&t)
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
                crate::ai::sanitize_tags(&t)
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
