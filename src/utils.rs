use std::{error::Error, fs, io::{self, Read, Write}, process::Command};
use tempfile::NamedTempFile;
use colored::*;
use crate::config::get_editor;

pub fn validate_content(content: &str) -> Result<(), String> {
    if content.trim().is_empty() {
        return Err("Content cannot be empty".into());
    }
    
    if content.len() > 1_000_000 {  // 1MB limit
        return Err("Content is too large (max 1MB)".into());
    }
    
    Ok(())
}

pub fn edit_content(initial: Option<&str>) -> Result<String, Box<dyn Error>> {
    let mut tmp = NamedTempFile::new()?;
    if let Some(s) = initial {
        tmp.write_all(s.as_bytes())?;
    }
    let p = tmp.path();

    let editor_cmd = get_editor();
    let mut parts = editor_cmd.split_whitespace();
    let command = parts.next().unwrap_or("vi");
    let args: Vec<&str> = parts.collect();

    let status = Command::new(command)
        .args(&args)
        .arg(p)
        .status()?;
    
    if !status.success() {
        return Err(format!("Editor '{}' exited with error", editor_cmd).into());
    }
    
    let mut buf = String::new();
    fs::File::open(p)?.read_to_string(&mut buf)?;
    
    if let Err(e) = validate_content(&buf) {
        return Err(e.into());
    }
    
    Ok(buf)
}

pub fn prompt_confirm(message: &str) -> bool {
    print!("{} {} [y/N]: ", "Confirm:".yellow().bold(), message);
    io::stdout().flush().ok();
    
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    
    input.trim().to_lowercase() == "y"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_content() {
        assert!(validate_content("hello").is_ok());
        assert!(validate_content("").is_err());
        assert!(validate_content("   ").is_err());
        
        let large = "a".repeat(1_000_001);
        assert!(validate_content(&large).is_err());
    }
}
