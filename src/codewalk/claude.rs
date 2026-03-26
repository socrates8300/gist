use crate::codewalk::types::{ApiConfig, ApiProvider, ConversationMessage, StreamEvent};
use futures_util::StreamExt;
use tokio::sync::mpsc;

/// Spawn a streaming API request in a background task.
/// Tokens are sent through `tx` as they arrive.
pub fn spawn_stream_request(
    api_config: ApiConfig,
    system_prompt: String,
    messages: Vec<ConversationMessage>,
    tx: mpsc::UnboundedSender<StreamEvent>,
) {
    tokio::spawn(async move {
        if let Err(e) = stream_request(&api_config, &system_prompt, &messages, &tx).await {
            let _ = tx.send(StreamEvent::Error(e.to_string()));
        }
    });
}

async fn stream_request(
    api_config: &ApiConfig,
    system_prompt: &str,
    messages: &[ConversationMessage],
    tx: &mpsc::UnboundedSender<StreamEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();

    match &api_config.provider {
        ApiProvider::Anthropic { api_key } => {
            stream_anthropic(&client, api_key, &api_config.model, system_prompt, messages, tx)
                .await
        }
        ApiProvider::OpenRouter { api_key, base_url } => {
            stream_openrouter(
                &client,
                api_key,
                base_url,
                &api_config.model,
                system_prompt,
                messages,
                tx,
            )
            .await
        }
    }
}

/// Stream from Anthropic Messages API (api.anthropic.com/v1/messages)
async fn stream_anthropic(
    client: &reqwest::Client,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    messages: &[ConversationMessage],
    tx: &mpsc::UnboundedSender<StreamEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let api_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content
            })
        })
        .collect();

    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "system": system_prompt,
        "messages": api_messages,
        "stream": true
    });

    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("Anthropic API error {}: {}", status, body_text).into());
    }

    parse_sse_stream(resp, tx, SseFormat::Anthropic).await
}

/// Stream from OpenRouter (chat/completions compatible)
async fn stream_openrouter(
    client: &reqwest::Client,
    api_key: &str,
    base_url: &str,
    model: &str,
    system_prompt: &str,
    messages: &[ConversationMessage],
    tx: &mpsc::UnboundedSender<StreamEvent>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut api_messages: Vec<serde_json::Value> = vec![serde_json::json!({
        "role": "system",
        "content": system_prompt
    })];
    for m in messages {
        api_messages.push(serde_json::json!({
            "role": m.role,
            "content": m.content
        }));
    }

    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": model,
        "messages": api_messages,
        "stream": true,
        "max_tokens": 4096
    });

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("OpenRouter API error {}: {}", status, body_text).into());
    }

    parse_sse_stream(resp, tx, SseFormat::OpenAI).await
}

#[derive(Debug)]
enum SseFormat {
    Anthropic,
    OpenAI,
}

/// Parse an SSE stream and send text tokens through the channel
async fn parse_sse_stream(
    resp: reqwest::Response,
    tx: &mpsc::UnboundedSender<StreamEvent>,
    format: SseFormat,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = resp.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        buffer.push_str(&chunk_str);

        // Process complete SSE lines
        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim_end_matches('\r').to_string();
            buffer = buffer[line_end + 1..].to_string();

            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                if data.trim() == "[DONE]" {
                    let _ = tx.send(StreamEvent::Done);
                    return Ok(());
                }

                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                    let text = match format {
                        SseFormat::Anthropic => extract_anthropic_text(&json),
                        SseFormat::OpenAI => extract_openai_text(&json),
                    };

                    if let Some(text) = text {
                        if !text.is_empty() {
                            let _ = tx.send(StreamEvent::Token(text));
                        }
                    }

                    // Check for Anthropic message_stop event
                    if matches!(format, SseFormat::Anthropic) {
                        if json.get("type").and_then(|t| t.as_str()) == Some("message_stop") {
                            let _ = tx.send(StreamEvent::Done);
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    // Stream ended without explicit [DONE]
    let _ = tx.send(StreamEvent::Done);
    Ok(())
}

/// Extract text from Anthropic SSE content_block_delta events
fn extract_anthropic_text(json: &serde_json::Value) -> Option<String> {
    let event_type = json.get("type")?.as_str()?;
    if event_type == "content_block_delta" {
        let delta = json.get("delta")?;
        if delta.get("type")?.as_str()? == "text_delta" {
            return delta.get("text")?.as_str().map(|s| s.to_string());
        }
    }
    None
}

/// Extract text from OpenAI-compatible SSE chunk events
fn extract_openai_text(json: &serde_json::Value) -> Option<String> {
    let choices = json.get("choices")?.as_array()?;
    let first = choices.first()?;
    let delta = first.get("delta")?;
    delta.get("content")?.as_str().map(|s| s.to_string())
}

/// Resolve API configuration from environment and config
pub fn resolve_api_config(
    model: &str,
    anthropic_key_config: Option<&str>,
    openrouter_key: Option<&str>,
    openrouter_base_url: Option<&str>,
) -> Result<ApiConfig, String> {
    // 1. Check ANTHROPIC_API_KEY env var
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            return Ok(ApiConfig {
                provider: ApiProvider::Anthropic { api_key: key },
                model: model.to_string(),
            });
        }
    }

    // 2. Check config anthropic_api_key
    if let Some(key) = anthropic_key_config {
        if !key.is_empty() {
            return Ok(ApiConfig {
                provider: ApiProvider::Anthropic {
                    api_key: key.to_string(),
                },
                model: model.to_string(),
            });
        }
    }

    // 3. Fall back to OpenRouter
    if let Some(key) = openrouter_key {
        if !key.is_empty() {
            let base_url = openrouter_base_url
                .unwrap_or("https://api.z.ai/api/coding/paas/v4")
                .to_string();
            return Ok(ApiConfig {
                provider: ApiProvider::OpenRouter {
                    api_key: key.to_string(),
                    base_url,
                },
                model: model.to_string(),
            });
        }
    }

    Err(
        "No API key found. Set ANTHROPIC_API_KEY environment variable, or configure \
         anthropic_api_key in config.toml, or set tag_api_key for OpenRouter fallback."
            .to_string(),
    )
}
