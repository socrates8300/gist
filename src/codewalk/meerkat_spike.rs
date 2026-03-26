//! Phase 0 spike: validate Meerkat integration with OpenRouter.
//!
//! This module is a throwaway proof-of-concept. It is entirely gated behind the
//! `meerkat` Cargo feature and has zero impact on the default build.
//!
//! Goal: confirm that
//!   1. Meerkat compiles into this binary
//!   2. A custom `LlmClient` backed by OpenRouter's Chat Completions API works
//!   3. Tool-call round-trips work (model calls `read_file`, gets content, responds)
//!
//! Run: `cargo run --features meerkat -- codewalk --meerkat-spike <path>`
//! or indirectly via `cargo test --features meerkat`.

#![allow(dead_code)]

use std::{path::PathBuf, sync::Arc};

use async_trait::async_trait;
use futures_util::StreamExt;
use meerkat::{
    AgentBuilder, AgentFactory, AgentToolDispatcher, JsonlStore, LlmClient, LlmDoneOutcome,
    LlmError, LlmEvent, LlmRequest, Message, StopReason, ToolDef, ToolError, ToolResult, Usage,
};
use meerkat_core::types::{ContentBlock, ContentInput, ToolCallView};
use meerkat_store::StoreAdapter;
use serde::Deserialize;
use serde_json::json;

// ── helpers ─────────────────────────────────────────────────────────────────

fn text_from_blocks(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ── OpenRouter Chat Completions client ──────────────────────────────────────

/// A minimal `LlmClient` that talks to any OpenAI-compatible Chat Completions
/// endpoint. Used here to bridge Meerkat's agent harness with OpenRouter.
pub struct OpenRouterChatClient {
    api_key: String,
    base_url: String,
}

impl OpenRouterChatClient {
    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
        }
    }
}

/// Convert Meerkat message history to OpenAI Chat Completions message array.
fn to_chat_messages(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .flat_map(|msg| -> Vec<serde_json::Value> {
            match msg {
                Message::System(s) => {
                    vec![json!({"role": "system", "content": s.content})]
                }
                Message::User(u) => {
                    vec![json!({"role": "user", "content": text_from_blocks(&u.content)})]
                }
                Message::Assistant(a) => {
                    let mut obj = json!({
                        "role": "assistant",
                        "content": if a.content.is_empty() {
                            serde_json::Value::Null
                        } else {
                            json!(a.content)
                        },
                    });
                    if !a.tool_calls.is_empty() {
                        obj["tool_calls"] = json!(a
                            .tool_calls
                            .iter()
                            .map(|tc| json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.args.to_string(),
                                }
                            }))
                            .collect::<Vec<_>>());
                    }
                    vec![obj]
                }
                Message::ToolResults { results } => results
                    .iter()
                    .map(|r| {
                        json!({
                            "role": "tool",
                            "tool_call_id": r.tool_use_id,
                            "content": text_from_blocks(&r.content),
                        })
                    })
                    .collect(),
                // BlockAssistant and future variants — skip for spike
                _ => vec![],
            }
        })
        .collect()
}

/// Convert Meerkat tool definitions to Chat Completions format.
fn to_chat_tools(tools: &[Arc<ToolDef>]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.input_schema,
                }
            })
        })
        .collect()
}

/// Make a single non-streaming Chat Completions call and return normalized events.
async fn do_chat_completion(
    api_key: &str,
    base_url: &str,
    request: LlmRequest,
) -> Result<Vec<LlmEvent>, LlmError> {
    let messages = to_chat_messages(&request.messages);
    let tools = to_chat_tools(&request.tools);

    let mut body = json!({
        "model": request.model,
        "messages": messages,
        "max_tokens": request.max_tokens,
        "stream": false,
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
    }
    if let Some(temp) = request.temperature {
        body["temperature"] = json!(temp);
    }

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/chat/completions", base_url))
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|_| LlmError::NetworkTimeout { duration_ms: 30_000 })?;

    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let msg = resp.text().await.unwrap_or_default();
        return Err(LlmError::from_http_status(status, msg));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| LlmError::Unknown { message: e.to_string() })?;

    let mut events = Vec::<LlmEvent>::new();
    let message = &data["choices"][0]["message"];

    // Tool calls — emit before text so the agent can dispatch them
    if let Some(tool_calls) = message["tool_calls"].as_array() {
        for tc in tool_calls {
            let id = tc["id"].as_str().unwrap_or("").to_string();
            let name = tc["function"]["name"].as_str().unwrap_or("").to_string();
            let args: serde_json::Value =
                serde_json::from_str(tc["function"]["arguments"].as_str().unwrap_or("{}"))
                    .unwrap_or_default();
            events.push(LlmEvent::ToolCallComplete { id, name, args, meta: None });
        }
    }

    // Text
    if let Some(text) = message["content"].as_str() {
        if !text.is_empty() {
            events.push(LlmEvent::TextDelta { delta: text.to_string(), meta: None });
        }
    }

    // Usage
    events.push(LlmEvent::UsageUpdate {
        usage: Usage {
            input_tokens: data["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            output_tokens: data["usage"]["completion_tokens"].as_u64().unwrap_or(0),
            cache_creation_tokens: None,
            cache_read_tokens: None,
        },
    });

    // Signal end-of-turn
    events.push(LlmEvent::Done {
        outcome: LlmDoneOutcome::Success { stop_reason: StopReason::EndTurn },
    });

    Ok(events)
}

#[async_trait]
impl LlmClient for OpenRouterChatClient {
    fn provider(&self) -> &'static str {
        "openrouter"
    }

    async fn health_check(&self) -> Result<(), LlmError> {
        Ok(())
    }

    fn stream<'a>(
        &'a self,
        request: &'a LlmRequest,
    ) -> std::pin::Pin<
        Box<dyn futures_util::Stream<Item = Result<LlmEvent, LlmError>> + Send + 'a>,
    > {
        let api_key = self.api_key.clone();
        let base_url = self.base_url.clone();
        let request = request.clone();

        Box::pin(
            futures_util::stream::once(async move {
                do_chat_completion(&api_key, &base_url, request).await
            })
            .flat_map(|result| {
                let events = match result {
                    Ok(evts) => evts.into_iter().map(Ok).collect::<Vec<_>>(),
                    Err(e) => vec![Err(e)],
                };
                futures_util::stream::iter(events)
            }),
        )
    }
}

// ── ReadFileTool ─────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ReadFileArgs {
    path: String,
}

/// Single-tool dispatcher for the spike. Reads files relative to `repo_path`.
pub struct ReadFileTool {
    pub repo_path: PathBuf,
}

#[async_trait]
impl AgentToolDispatcher for ReadFileTool {
    fn tools(&self) -> Arc<[Arc<ToolDef>]> {
        vec![Arc::new(ToolDef {
            name: "read_file".to_string(),
            description: "Read a source file from the repository. Returns its raw text content."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file, relative to the repository root"
                    }
                },
                "required": ["path"]
            }),
        })]
        .into()
    }

    async fn dispatch(&self, call: ToolCallView<'_>) -> Result<ToolResult, ToolError> {
        match call.name {
            "read_file" => {
                let args: ReadFileArgs = call
                    .parse_args::<ReadFileArgs>()
                    .map_err(|e: serde_json::Error| {
                        ToolError::invalid_arguments("read_file", e.to_string())
                    })?;

                let path = self.repo_path.join(&args.path);
                let content = std::fs::read_to_string(&path).map_err(|e| {
                    ToolError::invalid_arguments(
                        "read_file",
                        format!("cannot read {}: {e}", path.display()),
                    )
                })?;

                Ok(ToolResult::new(call.id.to_string(), content, false))
            }
            other => Err(ToolError::not_found(other)),
        }
    }
}

// ── Spike entry point ────────────────────────────────────────────────────────

/// Run the Phase 0 spike.
///
/// Reads `src/main.rs` from `repo_path` via the agent's `read_file` tool and
/// prints the model's summary. All output goes to stdout — this is a debug run,
/// not a TUI session.
pub async fn run_spike(
    api_key: &str,
    base_url: &str,
    model: &str,
    repo_path: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let _tmp = tempfile::TempDir::new()?;
    let store_dir = _tmp.path().to_path_buf();
    let factory = AgentFactory::new(store_dir.clone());

    let client: Arc<dyn LlmClient> =
        Arc::new(OpenRouterChatClient::new(api_key, base_url));
    let llm = factory.build_llm_adapter(client, model).await;

    let store = Arc::new(StoreAdapter::new(Arc::new(JsonlStore::new(
        factory.store_path.clone(),
    ))));
    let tools: Arc<dyn AgentToolDispatcher> =
        Arc::new(ReadFileTool { repo_path: repo_path.to_path_buf() });

    let mut agent = AgentBuilder::new()
        .model(model)
        .system_prompt(
            "You are a code reader. Use the read_file tool to read src/main.rs \
             and summarize what the program does. Do not guess — use the tool.",
        )
        .max_tokens_per_turn(2048)
        .build(Arc::new(llm), tools, store)
        .await;

    println!("[spike] model={model}  base_url={base_url}");
    let result = agent
        .run(ContentInput::Text(
            "Read src/main.rs using the read_file tool and explain what this program does."
                .to_string(),
        ))
        .await?;

    println!("[spike] === Agent response ===");
    println!("{}", result.text);
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_file_tool_exposes_one_tool() {
        let tool = ReadFileTool { repo_path: PathBuf::from(".") };
        let tools = tool.tools();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "read_file");
    }
}
