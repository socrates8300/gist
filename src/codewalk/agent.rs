//! Phase 2: Walk Agent — replaces the single-prompt stream with an agent that
//! can navigate the codebase using read_file, grep, and next_step tools.
//!
//! Architecture:
//!   spawn_walk_step() (sync) → tokio::spawn → run_walk_step() (async)
//!   → Meerkat agent → calls next_step → sends StreamEvent to TUI
//!
//! The TUI event loop is unchanged. `next_step` delivers steps via the same
//! `mpsc::UnboundedSender<StreamEvent>` used by the legacy claude.rs path.
//!
//! Gated via `#[cfg(feature = "meerkat")]` in mod.rs.

#![allow(dead_code)]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use async_trait::async_trait;
use meerkat::{
    AgentBuilder, AgentFactory, AgentToolDispatcher, JsonlStore, LlmClient, ToolDef, ToolError,
    ToolResult,
};
use meerkat_core::types::{ContentInput, ToolCallView};
use meerkat_store::StoreAdapter;
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use super::meerkat_spike::OpenRouterChatClient;
use super::types::{ApiConfig, ApiProvider, ConversationMessage, DeepDiveTag, StreamEvent};

// ── Tool dispatcher ───────────────────────────────────────────────────────────

struct WalkDispatcher {
    repo_path: PathBuf,
    tx: mpsc::UnboundedSender<StreamEvent>,
    step_delivered: Arc<AtomicBool>,
}

#[async_trait]
impl AgentToolDispatcher for WalkDispatcher {
    fn tools(&self) -> Arc<[Arc<ToolDef>]> {
        vec![
            Arc::new(ToolDef {
                name: "read_file".to_string(),
                description: "Read a source file from the repository. Path is relative to repo root."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "path": { "type": "string", "description": "Relative file path" }
                    },
                    "required": ["path"]
                }),
            }),
            Arc::new(ToolDef {
                name: "grep".to_string(),
                description: "Search for a pattern across the repository. Returns matching lines (up to 100)."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Regex or literal search pattern" },
                        "path":    { "type": "string", "description": "Optional subdirectory or file to search in" },
                        "include": { "type": "string", "description": "Optional file glob, e.g. '*.rs'" }
                    },
                    "required": ["pattern"]
                }),
            }),
            Arc::new(ToolDef {
                name: "task_note".to_string(),
                description: "Log an internal note about your reasoning (not shown to user)."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "note": { "type": "string" }
                    },
                    "required": ["note"]
                }),
            }),
            Arc::new(ToolDef {
                name: "next_step".to_string(),
                description: "Present the next walkthrough step to the user. Call exactly once when ready."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "file":        { "type": "string", "description": "Primary file (relative path, or OVERVIEW)" },
                        "line_start":  { "type": "integer", "description": "First relevant line (0 for overview)" },
                        "line_end":    { "type": "integer", "description": "Last relevant line (0 for overview)" },
                        "explanation": { "type": "string",  "description": "Full narrative explanation (markdown)" },
                        "deep_dives": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "id":    { "type": "string" },
                                    "label": { "type": "string" }
                                },
                                "required": ["id", "label"]
                            },
                            "description": "Topics worth exploring in detail (0-3 items)"
                        },
                        "next_file": { "type": "string", "description": "Next file to cover (omit if last step)" }
                    },
                    "required": ["file", "line_start", "line_end", "explanation"]
                }),
            }),
        ]
        .into()
    }

    async fn dispatch(&self, call: ToolCallView<'_>) -> Result<ToolResult, ToolError> {
        // Once next_step has fired, reject every subsequent tool call so the
        // Meerkat agent loop terminates instead of auto-advancing to the next file.
        if self.step_delivered.load(Ordering::SeqCst) {
            return Ok(ToolResult::new(
                call.id.to_string(),
                "STOP: the step has already been delivered. Do not call any more tools. \
                 The user will request the next step when they are ready."
                    .to_string(),
                true, // is_error — signals the agent to stop
            ));
        }

        match call.name {
            "read_file" => {
                #[derive(Deserialize)]
                struct Args {
                    path: String,
                }
                let args: Args = call
                    .parse_args::<Args>()
                    .map_err(|e: serde_json::Error| ToolError::invalid_arguments("read_file", e.to_string()))?;
                let path = self.repo_path.join(&args.path);
                let content = std::fs::read_to_string(&path).map_err(|e| {
                    ToolError::invalid_arguments(
                        "read_file",
                        format!("cannot read {}: {e}", path.display()),
                    )
                })?;
                Ok(ToolResult::new(call.id.to_string(), content, false))
            }

            "grep" => {
                #[derive(Deserialize)]
                struct Args {
                    pattern: String,
                    path: Option<String>,
                    include: Option<String>,
                }
                let args: Args = call
                    .parse_args::<Args>()
                    .map_err(|e: serde_json::Error| ToolError::invalid_arguments("grep", e.to_string()))?;

                let mut cmd = std::process::Command::new("grep");
                cmd.arg("-r").arg("-n").arg("--color=never");
                if let Some(inc) = &args.include {
                    cmd.arg(format!("--include={}", inc));
                }
                cmd.arg(&args.pattern);
                let search_target = args.path
                    .as_deref()
                    .map(|p| self.repo_path.join(p))
                    .unwrap_or_else(|| self.repo_path.clone());
                cmd.arg(&search_target);
                cmd.current_dir(&self.repo_path);

                let output = cmd
                    .output()
                    .map_err(|e| ToolError::invalid_arguments("grep", e.to_string()))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let limited: String = stdout.lines().take(100).collect::<Vec<_>>().join("\n");
                let result = if limited.is_empty() {
                    "No matches found.".to_string()
                } else {
                    limited
                };
                Ok(ToolResult::new(call.id.to_string(), result, false))
            }

            "task_note" => {
                // Internal logging — acknowledge silently
                Ok(ToolResult::new(call.id.to_string(), "Noted.".to_string(), false))
            }

            "next_step" => {
                #[derive(Deserialize)]
                struct Args {
                    file: String,
                    line_start: usize,
                    line_end: usize,
                    explanation: String,
                    #[serde(default)]
                    deep_dives: Vec<DeepDiveTag>,
                    next_file: Option<String>,
                }
                let args: Args = call
                    .parse_args::<Args>()
                    .map_err(|e: serde_json::Error| ToolError::invalid_arguments("next_step", e.to_string()))?;

                // Build the ClaudeStepResponse JSON envelope that the TUI expects.
                // The explanation goes AFTER the fence so parse_step_response picks it up.
                let meta = json!({
                    "file": args.file,
                    "line_start": args.line_start,
                    "line_end": args.line_end,
                    "deep_dives": args.deep_dives,
                    "next_file": args.next_file,
                });
                let content = format!(
                    "```json\n{}\n```\n\n{}",
                    serde_json::to_string_pretty(&meta).unwrap_or_default(),
                    args.explanation
                );

                let _ = self.tx.send(StreamEvent::Token(content));
                let _ = self.tx.send(StreamEvent::Done);
                self.step_delivered.store(true, Ordering::SeqCst);

                Ok(ToolResult::new(
                    call.id.to_string(),
                    "Step delivered. Your turn is complete — stop here. \
                     Do NOT call any more tools. The user will request the next step."
                        .to_string(),
                    false,
                ))
            }

            other => Err(ToolError::not_found(other)),
        }
    }
}

// ── Context builder ───────────────────────────────────────────────────────────

/// Build the user message for the walk agent from the conversation history.
fn build_step_message(
    conversation: &[ConversationMessage],
    step_number: usize,
    next_file_hint: Option<&str>,
) -> String {
    // Extract files already covered from assistant messages
    let covered: Vec<String> = conversation
        .iter()
        .filter(|m| m.role == "assistant")
        .filter_map(|m| {
            if let Some(start) = m.content.find("```json") {
                let after = &m.content[start + 7..];
                if let Some(end) = after.find("```") {
                    let json_str = after[..end].trim();
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                        return v["file"]
                            .as_str()
                            .filter(|f| *f != "OVERVIEW")
                            .map(|f| f.to_string());
                    }
                }
            }
            None
        })
        .collect();

    let mut parts = vec![format!("Produce step {} of the walkthrough.", step_number + 1)];
    if !covered.is_empty() {
        parts.push(format!("Already covered: {}", covered.join(", ")));
    }
    if let Some(hint) = next_file_hint {
        parts.push(format!("Suggested starting point: {}", hint));
    }
    parts.push(
        "Use read_file and grep to explore the codebase, then call next_step with your findings."
            .to_string(),
    );
    parts.join("\n")
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Spawn a walk agent step in a background task. Drop-in for `spawn_stream_request`.
///
/// The agent reads files, searches code, then calls `next_step` which sends
/// `StreamEvent::Token + StreamEvent::Done` through `tx` — identical to the
/// streaming path so the TUI event loop needs no changes.
pub fn spawn_walk_step(
    api_config: ApiConfig,
    system_prompt: String,
    conversation: Vec<ConversationMessage>,
    repo_path: PathBuf,
    tx: mpsc::UnboundedSender<StreamEvent>,
    step_number: usize,
    next_file_hint: Option<String>,
) {
    let tx_err = tx.clone();
    tokio::spawn(async move {
        if let Err(e) = run_walk_step(
            api_config,
            system_prompt,
            conversation,
            repo_path,
            tx,
            step_number,
            next_file_hint,
        )
        .await
        {
            let _ = tx_err.send(StreamEvent::Error(e.to_string()));
        }
    });
}

async fn run_walk_step(
    api_config: ApiConfig,
    system_prompt: String,
    conversation: Vec<ConversationMessage>,
    repo_path: PathBuf,
    tx: mpsc::UnboundedSender<StreamEvent>,
    step_number: usize,
    next_file_hint: Option<String>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (api_key, base_url) = match &api_config.provider {
        ApiProvider::OpenRouter { api_key, base_url } => (api_key.clone(), base_url.clone()),
        ApiProvider::Anthropic { .. } => {
            return Err("Walk agent requires OpenRouter (Anthropic not supported yet)".into());
        }
    };

    let tmp = tempfile::TempDir::new()?;
    let factory = AgentFactory::new(tmp.path().to_path_buf());

    let client: Arc<dyn LlmClient> = Arc::new(OpenRouterChatClient::new(&api_key, &base_url));
    let llm = factory.build_llm_adapter(client, &api_config.model).await;
    let store = Arc::new(StoreAdapter::new(Arc::new(JsonlStore::new(
        factory.store_path.clone(),
    ))));

    let step_delivered = Arc::new(AtomicBool::new(false));
    let dispatcher = Arc::new(WalkDispatcher {
        repo_path,
        tx: tx.clone(),
        step_delivered: Arc::clone(&step_delivered),
    }) as Arc<dyn AgentToolDispatcher>;

    let mut agent = AgentBuilder::new()
        .model(&api_config.model)
        .system_prompt(&system_prompt)
        .max_tokens_per_turn(4096)
        .build(Arc::new(llm), dispatcher, store)
        .await;

    let user_msg = build_step_message(&conversation, step_number, next_file_hint.as_deref());
    let run_result = agent.run(ContentInput::Text(user_msg)).await?;

    drop(tmp);

    // If the agent ended without calling next_step (e.g. max tokens), deliver its
    // text output as a fallback so the TUI doesn't hang.
    if !step_delivered.load(Ordering::SeqCst) {
        let text = run_result.text;
        if !text.is_empty() {
            let _ = tx.send(StreamEvent::Token(text));
        }
        let _ = tx.send(StreamEvent::Done);
    }

    Ok(())
}
