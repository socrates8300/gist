use serde::{Deserialize, Serialize};
use std::error::Error;
use crate::config::Config;

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

pub fn sanitize_tags(tags: &str) -> String {
    tags.split(',')
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .take(10)  // Limit to 10 tags
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn get_tags(content: &str, config: &Config) -> Result<String, Box<dyn Error>> {
    // Skip if auto-generate is disabled
    if !config.auto_generate_tags {
        return Ok(config.default_tags.join(", "));
    }

    // Try using API if key is available
    if let Some(key) = &config.tag_api_key {
        let model = config.ai_model.as_deref().unwrap_or("openai/gpt-4o");
        let base_url = config.ai_base_url.as_deref().unwrap_or("https://openrouter.ai/api/v1");
        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

        let reqbody = serde_json::json!({
            "model": model,
            "messages": [{"role":"user","content":format!("Extract 3-5 relevant tags separated by commas:\n{}",content)}],
            "temperature": 0.1,
        });
        
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
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
