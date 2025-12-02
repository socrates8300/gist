use serde::{Deserialize, Serialize};
use std::{env, error::Error, fs, path::PathBuf, process::Command};
use crate::models::Theme;

#[derive(Deserialize, Serialize, Default, Clone)]
pub struct Config {
    pub editor: String,
    pub default_tags: Vec<String>,
    pub theme: Theme,
    pub auto_generate_tags: bool,
    pub tag_api_key: Option<String>,
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

pub fn save_config(config: &Config) -> Result<(), Box<dyn Error>> {
    let config_path = get_config_path()?;
    let json_str = serde_json::to_string_pretty(config)?;
    fs::write(config_path, json_str)?;
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
