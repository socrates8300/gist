use serde::{Deserialize, Serialize};
use std::{env, error::Error, fs, path::PathBuf, process::Command};
use crate::models::Theme;

#[derive(Deserialize, Serialize, Clone)]
pub struct CodewalkConfig {
    #[serde(default = "default_true")]
    pub enable_memory: bool,
    #[serde(default = "default_compaction_threshold")]
    pub compaction_threshold: usize,
    #[serde(default = "default_retention_days")]
    pub session_retention_days: u32,
    // Phase 4: budget controls (deep-audit)
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_max_tool_calls")]
    pub max_tool_calls: usize,
    #[serde(default = "default_max_wall_seconds")]
    pub max_wall_seconds: u64,
    #[serde(default = "default_max_subagents")]
    pub max_subagents: usize,
    // Recon agent budget (separate from deep-audit)
    #[serde(default = "default_recon_max_tool_calls")]
    pub recon_max_tool_calls: usize,
    #[serde(default = "default_recon_max_wall_seconds")]
    pub recon_max_wall_seconds: u64,
}

fn default_true() -> bool { true }
fn default_compaction_threshold() -> usize { 50 }
fn default_retention_days() -> u32 { 30 }
fn default_max_tokens() -> usize { 100_000 }
fn default_max_tool_calls() -> usize { 200 }
fn default_max_wall_seconds() -> u64 { 300 }
fn default_max_subagents() -> usize { 4 }
fn default_recon_max_tool_calls() -> usize { 100 }
fn default_recon_max_wall_seconds() -> u64 { 300 }

impl Default for CodewalkConfig {
    fn default() -> Self {
        Self {
            enable_memory: true,
            compaction_threshold: 50,
            session_retention_days: 30,
            max_tokens: 100_000,
            max_tool_calls: 200,
            max_wall_seconds: 300,
            max_subagents: 4,
            recon_max_tool_calls: 100,
            recon_max_wall_seconds: 300,
        }
    }
}

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub editor: String,
    pub default_tags: Vec<String>,
    pub theme: Theme,
    pub auto_generate_tags: bool,
    pub tag_api_key: Option<String>,
    pub ai_model: Option<String>,
    pub ai_base_url: Option<String>,
    pub anthropic_api_key: Option<String>,
    #[serde(default)]
    pub codewalk: Option<CodewalkConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            editor: String::new(),
            default_tags: vec!["snippet".to_string()],
            theme: Theme::Dark,
            auto_generate_tags: true,
            tag_api_key: None,
            ai_model: Some("glm-5-turbo".to_string()),
            ai_base_url: Some("https://api.z.ai/api/coding/paas/v4".to_string()),
            anthropic_api_key: None,
            codewalk: None,
        }
    }
}

pub fn get_gist_dir() -> Result<PathBuf, Box<dyn Error>> {
    let home = dirs::home_dir().ok_or("Could not determine home directory")?;
    let dir = home.join(".config").join("gist");

    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn get_config_path() -> Result<PathBuf, Box<dyn Error>> {
    Ok(get_gist_dir()?.join("config.toml"))
}

pub fn load_config() -> Config {
    if let Ok(config_path) = get_config_path() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            // Try TOML first
            if let Ok(config) = toml::from_str::<Config>(&content) {
                return config;
            }
            // Fallback to JSON (migration)
            if let Ok(config) = serde_json::from_str::<Config>(&content) {
                // Save as TOML immediately to migrate
                let _ = save_config(&config);
                return config;
            }
        }
    }
    
    // Create default config if loading fails
    let default_config = Config::default();
    
    // Try to save default config
    if let Ok(config_path) = get_config_path() {
        if let Ok(toml_str) = toml::to_string_pretty(&default_config) {
            let _ = fs::write(config_path, toml_str);
        }
    }
    
    default_config
}

pub fn save_config(config: &Config) -> Result<(), Box<dyn Error>> {
    let config_path = get_config_path()?;
    let toml_str = toml::to_string_pretty(config)?;
    fs::write(config_path, toml_str)?;
    Ok(())
}

pub fn get_editor() -> String {
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
