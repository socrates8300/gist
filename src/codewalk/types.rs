use serde::{Deserialize, Serialize};

/// Claude's structured step response parsed from the JSON envelope
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeStepResponse {
    pub file: String,
    pub line_start: usize,
    pub line_end: usize,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub deep_dives: Vec<DeepDiveTag>,
    pub next_file: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeepDiveTag {
    pub id: String,
    pub label: String,
}

/// A completed walkthrough step (includes cached file content)
#[derive(Debug, Clone)]
pub struct WalkStep {
    pub index: usize,
    pub response: ClaudeStepResponse,
    pub file_content: String,
    pub is_deep_dive: bool,
    pub parent_step: Option<usize>,
}

/// A tech debt note entered by the user
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TechDebtNote {
    pub file: String,
    pub line_range: String,
    pub note: String,
    pub timestamp: String,
}

/// A message in the Claude conversation history
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}

/// Events received from the streaming API task
#[derive(Debug)]
pub enum StreamEvent {
    Token(String),
    Done,
    Error(String),
}

/// Which API provider to use
#[derive(Debug, Clone)]
pub enum ApiProvider {
    Anthropic { api_key: String },
    OpenRouter { api_key: String, base_url: String },
}

/// Full API configuration for a session
#[derive(Debug, Clone)]
pub struct ApiConfig {
    pub provider: ApiProvider,
    pub model: String,
}

// ── Walk mode ────────────────────────────────────────────────────────────────

/// Controls the walk agent's focus and system prompt variant
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub enum WalkMode {
    #[default]
    Onboarding,
    Review,
    Audit,
    Security,
}

impl std::fmt::Display for WalkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WalkMode::Onboarding => write!(f, "onboarding"),
            WalkMode::Review => write!(f, "review"),
            WalkMode::Audit => write!(f, "audit"),
            WalkMode::Security => write!(f, "security"),
        }
    }
}

impl std::str::FromStr for WalkMode {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "onboarding" => Ok(WalkMode::Onboarding),
            "review" => Ok(WalkMode::Review),
            "audit" => Ok(WalkMode::Audit),
            "security" => Ok(WalkMode::Security),
            other => Err(format!(
                "Unknown mode '{}'. Use: onboarding, review, audit, security",
                other
            )),
        }
    }
}

// ── Recon types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoMap {
    pub entry_points: Vec<String>,
    pub key_modules: Vec<ModuleSummary>,
    pub dependency_edges: Vec<(String, String)>,
    pub recent_changes: Vec<CommitSummary>,
    pub estimated_complexity: Complexity,
    pub suggested_walk_order: Vec<String>,
    pub repo_stats: RepoStats,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModuleSummary {
    pub path: String,
    pub purpose: String,
    pub key_exports: Vec<String>,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommitSummary {
    pub hash: String,
    pub message: String,
    pub date: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Complexity {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RepoStats {
    pub file_count: usize,
    pub approx_loc: usize,
}
