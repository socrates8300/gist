use serde::{Deserialize, Serialize};
use std::{env, error::Error, fs, path::PathBuf, process::Command};
use crate::models::Theme;

#[derive(Deserialize, Serialize, Clone)]
pub struct Config {
    pub editor: String,
    pub default_tags: Vec<String>,
    pub theme: Theme,
    pub auto_generate_tags: bool,
    pub tag_api_key: Option<String>,
    pub ai_model: Option<String>,
    pub ai_base_url: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            editor: String::new(),
            default_tags: vec!["snippet".to_string()],
            theme: Theme::Dark,
            auto_generate_tags: true,
            tag_api_key: None,
            ai_model: Some("openai/gpt-4o".to_string()),
            ai_base_url: Some("https://openrouter.ai/api/v1".to_string()),
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
