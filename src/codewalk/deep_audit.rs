//! Phase 4: Deep audit — parallel per-module sub-agent analysis.
//!
//! `run_deep_audit` fans out one sub-agent per module (up to `max_subagents`
//! concurrently via a semaphore). Each sub-agent uses read_file, grep, and a
//! `submit_findings` terminal tool. Budget is enforced shared across all
//! sub-agents via atomic counters checked before every tool dispatch.
//!
//! Gated by `#[cfg(feature = "meerkat")]` in mod.rs.

#![allow(dead_code)]

use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Instant,
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
use tokio::sync::{Mutex, Semaphore};

use super::meerkat_spike::OpenRouterChatClient;
use super::types::{ApiConfig, ApiProvider, BudgetConfig, FileRef, ModuleFindings, ModuleSummary};

// ── Budget state ──────────────────────────────────────────────────────────────

/// Shared budget state across all sub-agents in one deep-audit run.
pub struct BudgetState {
    pub total_tool_calls: Arc<AtomicUsize>,
    pub budget_exceeded: Arc<AtomicBool>,
    pub start: Instant,
    pub config: BudgetConfig,
}

impl BudgetState {
    pub fn new(config: BudgetConfig) -> Arc<Self> {
        Arc::new(Self {
            total_tool_calls: Arc::new(AtomicUsize::new(0)),
            budget_exceeded: Arc::new(AtomicBool::new(false)),
            start: Instant::now(),
            config,
        })
    }

    /// Returns Some(reason) if any budget limit has been exceeded.
    pub fn check(&self) -> Option<&'static str> {
        if self.budget_exceeded.load(Ordering::SeqCst) {
            return Some("budget already exceeded");
        }
        if self.total_tool_calls.load(Ordering::SeqCst) >= self.config.max_tool_calls {
            self.budget_exceeded.store(true, Ordering::SeqCst);
            return Some("tool call limit reached");
        }
        if self.start.elapsed().as_secs() >= self.config.max_wall_seconds {
            self.budget_exceeded.store(true, Ordering::SeqCst);
            return Some("wall clock time limit reached");
        }
        None
    }

    pub fn increment_tool_calls(&self) {
        self.total_tool_calls.fetch_add(1, Ordering::SeqCst);
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.start.elapsed().as_secs()
    }
}

// ── Per-module dispatcher ─────────────────────────────────────────────────────

struct ModuleAuditDispatcher {
    repo_path: PathBuf,
    findings: Arc<Mutex<Option<ModuleFindings>>>,
    budget: Arc<BudgetState>,
    module_tool_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl AgentToolDispatcher for ModuleAuditDispatcher {
    fn tools(&self) -> Arc<[Arc<ToolDef>]> {
        vec![
            Arc::new(ToolDef {
                name: "read_file".to_string(),
                description: "Read a source file from the repository (first 200 lines).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "path": { "type": "string", "description": "Relative file path" } },
                    "required": ["path"]
                }),
            }),
            Arc::new(ToolDef {
                name: "grep".to_string(),
                description: "Search for a pattern across the repository (up to 50 matches).".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "pattern": { "type": "string" },
                        "path":    { "type": "string" },
                        "include": { "type": "string", "description": "Glob filter, e.g. '*.rs'" }
                    },
                    "required": ["pattern"]
                }),
            }),
            Arc::new(ToolDef {
                name: "submit_findings".to_string(),
                description: "Submit your complete module audit. Call exactly once when done.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "module_path": { "type": "string" },
                        "purpose":     { "type": "string" },
                        "findings": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Key observations (bullet points)"
                        },
                        "risks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Potential issues or technical risks"
                        },
                        "file_refs": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": { "type": "string" },
                                    "line": { "type": "integer" },
                                    "note": { "type": "string" }
                                },
                                "required": ["path", "line", "note"]
                            },
                            "description": "Specific file:line references for findings"
                        }
                    },
                    "required": ["module_path", "purpose", "findings", "risks"]
                }),
            }),
        ]
        .into()
    }

    async fn dispatch(&self, call: ToolCallView<'_>) -> Result<ToolResult, ToolError> {
        // Budget check before every tool execution
        if let Some(reason) = self.budget.check() {
            return Ok(ToolResult::new(
                call.id.to_string(),
                format!(
                    "BUDGET_EXCEEDED: {}. Call submit_findings now with whatever you have.",
                    reason
                ),
                true,
            ));
        }
        self.budget.increment_tool_calls();
        self.module_tool_calls.fetch_add(1, Ordering::SeqCst);

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
                // Limit to 200 lines to conserve tokens per sub-agent
                let truncated: String = content.lines().take(200).collect::<Vec<_>>().join("\n");
                Ok(ToolResult::new(call.id.to_string(), truncated, false))
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
                let target = args
                    .path
                    .as_deref()
                    .map(|p| self.repo_path.join(p))
                    .unwrap_or_else(|| self.repo_path.clone());
                cmd.arg(&target).current_dir(&self.repo_path);
                let output = cmd
                    .output()
                    .map_err(|e| ToolError::invalid_arguments("grep", e.to_string()))?;
                let stdout = String::from_utf8_lossy(&output.stdout);
                let limited: String = stdout.lines().take(50).collect::<Vec<_>>().join("\n");
                Ok(ToolResult::new(
                    call.id.to_string(),
                    if limited.is_empty() { "No matches.".to_string() } else { limited },
                    false,
                ))
            }

            "submit_findings" => {
                #[derive(Deserialize)]
                struct Args {
                    module_path: String,
                    purpose: String,
                    findings: Vec<String>,
                    risks: Vec<String>,
                    #[serde(default)]
                    file_refs: Vec<FileRef>,
                }
                let args: Args = call
                    .parse_args::<Args>()
                    .map_err(|e: serde_json::Error| ToolError::invalid_arguments("submit_findings", e.to_string()))?;
                let tool_calls = self.module_tool_calls.load(Ordering::SeqCst);
                *self.findings.lock().await = Some(ModuleFindings {
                    module_path: args.module_path,
                    purpose: args.purpose,
                    findings: args.findings,
                    risks: args.risks,
                    file_refs: args.file_refs,
                    tool_calls_used: tool_calls,
                });
                Ok(ToolResult::new(
                    call.id.to_string(),
                    "Findings submitted.".to_string(),
                    false,
                ))
            }

            other => Err(ToolError::not_found(other)),
        }
    }
}

// ── Single module audit ───────────────────────────────────────────────────────

async fn audit_one_module(
    api_config: ApiConfig,
    module: ModuleSummary,
    repo_path: PathBuf,
    budget: Arc<BudgetState>,
) -> ModuleFindings {
    // Short-circuit if budget already blown before we start
    if budget.check().is_some() {
        return ModuleFindings {
            module_path: module.path,
            purpose: module.purpose,
            findings: vec!["Skipped: budget exhausted before this module was reached.".to_string()],
            risks: vec![],
            file_refs: vec![],
            tool_calls_used: 0,
        };
    }

    let (api_key, base_url) = match &api_config.provider {
        ApiProvider::OpenRouter { api_key, base_url } => (api_key.clone(), base_url.clone()),
        ApiProvider::Anthropic { .. } => {
            return ModuleFindings {
                module_path: module.path,
                purpose: module.purpose,
                findings: vec!["Error: Anthropic not supported for sub-agents (use OpenRouter).".to_string()],
                risks: vec![],
                file_refs: vec![],
                tool_calls_used: 0,
            };
        }
    };

    let tmp = match tempfile::TempDir::new() {
        Ok(t) => t,
        Err(e) => {
            return ModuleFindings {
                module_path: module.path,
                purpose: module.purpose,
                findings: vec![format!("Error: failed to create temp dir: {e}")],
                risks: vec![],
                file_refs: vec![],
                tool_calls_used: 0,
            };
        }
    };

    let factory = AgentFactory::new(tmp.path().to_path_buf());
    let client: Arc<dyn LlmClient> = Arc::new(OpenRouterChatClient::new(&api_key, &base_url));
    let llm = factory.build_llm_adapter(client, &api_config.model).await;
    let store = Arc::new(StoreAdapter::new(Arc::new(JsonlStore::new(
        factory.store_path.clone(),
    ))));

    let findings_cell: Arc<Mutex<Option<ModuleFindings>>> = Arc::new(Mutex::new(None));
    let module_tool_calls = Arc::new(AtomicUsize::new(0));
    let dispatcher = Arc::new(ModuleAuditDispatcher {
        repo_path,
        findings: Arc::clone(&findings_cell),
        budget: Arc::clone(&budget),
        module_tool_calls: Arc::clone(&module_tool_calls),
    }) as Arc<dyn AgentToolDispatcher>;

    let system_prompt = format!(
        "You are an expert code auditor analyzing one module in a larger codebase.\n\
         Module path: {path}\n\
         Purpose: {purpose}\n\
         Depends on: {deps}\n\n\
         Instructions:\n\
         1. Use read_file to read the main file(s) in this module (path relative to repo root)\n\
         2. Use grep to check for error-handling patterns, unsafe code, and external calls\n\
         3. Identify findings (key observations) and risks (potential problems)\n\
         4. Include specific file:line references where relevant\n\
         5. Call submit_findings with your complete analysis\n\n\
         Be specific. Note any missing error handling, security concerns, or tight coupling.\n\
         If you receive a BUDGET_EXCEEDED message, call submit_findings immediately.",
        path = module.path,
        purpose = module.purpose,
        deps = if module.depends_on.is_empty() {
            "none".to_string()
        } else {
            module.depends_on.join(", ")
        },
    );

    let mut agent = AgentBuilder::new()
        .model(&api_config.model)
        .system_prompt(&system_prompt)
        .max_tokens_per_turn(2048)
        .build(Arc::new(llm), dispatcher, store)
        .await;

    let prompt = format!(
        "Audit the module at '{}'. Read its files, assess risks, then call submit_findings.",
        module.path
    );
    let _ = agent.run(ContentInput::Text(prompt)).await;
    drop(tmp);

    let tool_calls_used = module_tool_calls.load(Ordering::SeqCst);
    let result = findings_cell.lock().await.take();
    result.unwrap_or_else(|| ModuleFindings {
        module_path: module.path,
        purpose: module.purpose,
        findings: vec![
            "Agent completed without calling submit_findings (possible budget exhaustion or model error).".to_string(),
        ],
        risks: vec![],
        file_refs: vec![],
        tool_calls_used,
    })
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Run parallel module audits, respecting `max_subagents` concurrency.
/// Returns findings in the order modules were provided (not completion order).
pub async fn run_deep_audit(
    api_config: &ApiConfig,
    modules: Vec<ModuleSummary>,
    repo_path: &std::path::Path,
    budget: Arc<BudgetState>,
) -> Vec<ModuleFindings> {
    let semaphore = Arc::new(Semaphore::new(budget.config.max_subagents));
    let repo_path = repo_path.to_path_buf();

    let mut handles = Vec::with_capacity(modules.len());
    for module in modules {
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .expect("semaphore closed");
        let api_config = api_config.clone();
        let repo_path = repo_path.clone();
        let budget = Arc::clone(&budget);
        handles.push(tokio::spawn(async move {
            let _permit = permit; // held for the duration of this module's audit
            audit_one_module(api_config, module, repo_path, budget).await
        }));
    }

    let mut results = Vec::with_capacity(handles.len());
    for handle in handles {
        match handle.await {
            Ok(findings) => results.push(findings),
            Err(e) => results.push(ModuleFindings {
                module_path: "unknown".to_string(),
                purpose: String::new(),
                findings: vec![format!("Sub-agent task panicked: {e}")],
                risks: vec![],
                file_refs: vec![],
                tool_calls_used: 0,
            }),
        }
    }
    results
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Format a `ModuleFindings` as the markdown explanation shown in the TUI.
pub fn format_findings_as_explanation(f: &ModuleFindings) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {}\n\n", f.module_path));
    out.push_str(&format!("**Purpose:** {}\n\n", f.purpose));

    if !f.findings.is_empty() {
        out.push_str("### Findings\n\n");
        for item in &f.findings {
            out.push_str(&format!("- {}\n", item));
        }
        out.push('\n');
    }

    if !f.risks.is_empty() {
        out.push_str("### Risks\n\n");
        for risk in &f.risks {
            out.push_str(&format!("- {}\n", risk));
        }
        out.push('\n');
    }

    if !f.file_refs.is_empty() {
        out.push_str("### File References\n\n");
        for r in &f.file_refs {
            out.push_str(&format!("- `{}:{}` — {}\n", r.path, r.line, r.note));
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "_Tool calls used: {}_\n",
        f.tool_calls_used
    ));
    out
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codewalk::types::BudgetConfig;

    #[test]
    fn budget_state_tool_call_limit() {
        let budget = BudgetState::new(BudgetConfig {
            max_tool_calls: 2,
            max_wall_seconds: 3600,
            max_subagents: 4,
            max_tokens: 100_000,
        });
        assert!(budget.check().is_none());
        budget.increment_tool_calls();
        budget.increment_tool_calls();
        assert!(budget.check().is_some());
    }

    #[test]
    fn budget_state_propagates_exceeded_flag() {
        let budget = BudgetState::new(BudgetConfig {
            max_tool_calls: 1,
            max_wall_seconds: 3600,
            max_subagents: 4,
            max_tokens: 100_000,
        });
        budget.increment_tool_calls();
        // First call trips the limit and sets the flag
        let _ = budget.check();
        // Second call should immediately return from the flag
        assert!(budget.check().is_some());
    }

    #[test]
    fn format_findings_contains_module_path() {
        let f = ModuleFindings {
            module_path: "src/foo.rs".to_string(),
            purpose: "does stuff".to_string(),
            findings: vec!["thing A".to_string()],
            risks: vec!["risk B".to_string()],
            file_refs: vec![FileRef {
                path: "src/foo.rs".to_string(),
                line: 42,
                note: "suspicious unwrap".to_string(),
            }],
            tool_calls_used: 3,
        };
        let text = format_findings_as_explanation(&f);
        assert!(text.contains("src/foo.rs"));
        assert!(text.contains("thing A"));
        assert!(text.contains("42"));
    }

    #[test]
    fn module_audit_dispatcher_exposes_three_tools() {
        use tokio::sync::Mutex;
        let d = ModuleAuditDispatcher {
            repo_path: PathBuf::from("."),
            findings: Arc::new(Mutex::new(None)),
            budget: BudgetState::new(BudgetConfig::default()),
            module_tool_calls: Arc::new(AtomicUsize::new(0)),
        };
        assert_eq!(d.tools().len(), 3);
        let tools = d.tools();
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"read_file"));
        assert!(names.contains(&"grep"));
        assert!(names.contains(&"submit_findings"));
    }
}
