use crate::codewalk::types::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Input modes for the CodeWalk TUI
#[derive(Debug, Clone, PartialEq)]
pub enum CWInputMode {
    Normal,
    NoteInput,
    Help,
    DeepDiveList,
    SearchInFile,
    ConfirmQuit,
    WaitingForStep,
}

/// Which panel is focused
#[derive(Debug, Clone, PartialEq)]
pub enum CWPanel {
    Code,
    Explanation,
}

/// Main application state for a CodeWalk session
pub struct CodeWalkApp {
    // Step navigation
    pub steps: Vec<WalkStep>,
    pub current_step: usize,

    // Panel state
    pub focused_panel: CWPanel,
    pub code_scroll: u16,
    pub explanation_scroll: u16,

    // Streaming state
    pub streaming_text: String,
    pub is_streaming: bool,

    // Deep dives
    pub all_deep_dives: Vec<(usize, DeepDiveTag)>,
    pub deep_dive_cursor: usize,

    // Tech debt
    pub tech_debt_notes: Vec<TechDebtNote>,
    pub tech_debt_visible: bool,
    pub note_input_buffer: String,

    // UI state
    pub mode: CWInputMode,
    pub status_message: Option<(String, Instant)>,
    pub search_query: String,
    pub search_results: Vec<usize>,

    // Session info
    pub scope: String,
    pub repo_path: PathBuf,
    pub conversation: Vec<ConversationMessage>,
    pub overview_text: String,

    // Pending `g` for `gg` command
    pub pending_g: bool,

    // Should quit
    pub should_quit: bool,

    // Output path for session export
    pub output_path: Option<PathBuf>,
}

impl CodeWalkApp {
    pub fn new(scope: String, repo_path: PathBuf, output_path: Option<PathBuf>) -> Self {
        Self {
            steps: Vec::new(),
            current_step: 0,
            focused_panel: CWPanel::Explanation,
            code_scroll: 0,
            explanation_scroll: 0,
            streaming_text: String::new(),
            is_streaming: false,
            all_deep_dives: Vec::new(),
            deep_dive_cursor: 0,
            tech_debt_notes: Vec::new(),
            tech_debt_visible: false,
            note_input_buffer: String::new(),
            mode: CWInputMode::WaitingForStep,
            status_message: None,
            search_query: String::new(),
            search_results: Vec::new(),
            scope,
            repo_path,
            conversation: Vec::new(),
            overview_text: String::new(),
            pending_g: false,
            should_quit: false,
            output_path,
        }
    }

    pub fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }

    pub fn get_status(&self) -> Option<&str> {
        if let Some((msg, time)) = &self.status_message {
            if time.elapsed() < Duration::from_secs(5) {
                return Some(msg.as_str());
            }
        }
        None
    }

    pub fn current_step_data(&self) -> Option<&WalkStep> {
        self.steps.get(self.current_step)
    }

    /// Get the current file path being displayed
    pub fn current_file(&self) -> Option<&str> {
        self.current_step_data()
            .map(|s| s.response.file.as_str())
    }

    /// Get the current explanation text (either streaming or completed)
    pub fn current_explanation(&self) -> &str {
        if self.is_streaming {
            &self.streaming_text
        } else if let Some(step) = self.current_step_data() {
            &step.response.explanation
        } else {
            ""
        }
    }

    /// Get the current code content
    pub fn current_code(&self) -> &str {
        if let Some(step) = self.current_step_data() {
            &step.file_content
        } else {
            ""
        }
    }

    /// Get highlighted line range for code panel
    pub fn highlight_range(&self) -> Option<(usize, usize)> {
        self.current_step_data()
            .map(|s| (s.response.line_start, s.response.line_end))
    }

    /// Navigate to the next step (returns true if we need to request a new step from Claude)
    pub fn go_next(&mut self) -> bool {
        if self.is_streaming {
            return false;
        }
        if self.current_step + 1 < self.steps.len() {
            self.current_step += 1;
            self.reset_scrolls();
            false
        } else {
            // Need to request a new step
            true
        }
    }

    /// Navigate to the previous step
    pub fn go_prev(&mut self) {
        if self.is_streaming {
            return;
        }
        if self.current_step > 0 {
            self.current_step -= 1;
            self.reset_scrolls();
        }
    }

    /// Jump forward N steps (returns true if reached the end and needs new step)
    pub fn jump_forward(&mut self, n: usize) -> bool {
        if self.is_streaming {
            return false;
        }
        let target = self.current_step + n;
        if target < self.steps.len() {
            self.current_step = target;
            self.reset_scrolls();
            false
        } else if !self.steps.is_empty() {
            self.current_step = self.steps.len() - 1;
            self.reset_scrolls();
            // Request new step only if we were trying to go beyond
            target >= self.steps.len()
        } else {
            true
        }
    }

    /// Jump back N steps
    pub fn jump_back(&mut self, n: usize) {
        if self.is_streaming {
            return;
        }
        self.current_step = self.current_step.saturating_sub(n);
        self.reset_scrolls();
    }

    /// Jump to the first step
    pub fn go_start(&mut self) {
        if self.is_streaming {
            return;
        }
        self.current_step = 0;
        self.reset_scrolls();
    }

    /// Jump to the last step
    pub fn go_end(&mut self) {
        if self.is_streaming {
            return;
        }
        if !self.steps.is_empty() {
            self.current_step = self.steps.len() - 1;
            self.reset_scrolls();
        }
    }

    /// Get deep dives available at the current step
    pub fn current_deep_dives(&self) -> Vec<&DeepDiveTag> {
        self.current_step_data()
            .map(|s| s.response.deep_dives.iter().collect())
            .unwrap_or_default()
    }

    /// Add a tech debt note for the current step
    pub fn add_tech_debt_note(&mut self, note: String) {
        let (file, line_range) = if let Some(step) = self.current_step_data() {
            (
                step.response.file.clone(),
                format!("{}-{}", step.response.line_start, step.response.line_end),
            )
        } else {
            ("unknown".to_string(), "0-0".to_string())
        };

        self.tech_debt_notes.push(TechDebtNote {
            file,
            line_range,
            note,
            timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
        });
    }

    /// Parse a completed streaming response into a WalkStep
    pub fn finalize_step(&mut self, file_content: String) {
        let raw_text = std::mem::take(&mut self.streaming_text);

        // Parse the JSON envelope from the beginning of the response
        let (response, explanation) = parse_step_response(&raw_text);

        let mut response = response;
        response.explanation = explanation;

        // Collect deep dives
        let step_index = self.steps.len();
        for dd in &response.deep_dives {
            self.all_deep_dives.push((step_index, dd.clone()));
        }

        // Store the overview if this is the first step
        if self.steps.is_empty() && response.file == "OVERVIEW" {
            self.overview_text = response.explanation.clone();
        }

        let step = WalkStep {
            index: step_index,
            response,
            file_content,
            is_deep_dive: false,
            parent_step: None,
        };

        self.steps.push(step);
        self.current_step = self.steps.len() - 1;
        self.is_streaming = false;
        self.mode = CWInputMode::Normal;
        self.reset_scrolls();
    }

    /// Start streaming mode for a new step
    pub fn start_streaming(&mut self) {
        self.streaming_text.clear();
        self.is_streaming = true;
        self.mode = CWInputMode::WaitingForStep;
    }

    /// Toggle panel focus
    pub fn toggle_panel(&mut self) {
        self.focused_panel = match self.focused_panel {
            CWPanel::Code => CWPanel::Explanation,
            CWPanel::Explanation => CWPanel::Code,
        };
    }

    fn reset_scrolls(&mut self) {
        self.code_scroll = 0;
        self.explanation_scroll = 0;
    }

    /// Approximate token count of conversation history
    pub fn approx_token_count(&self) -> usize {
        self.conversation
            .iter()
            .map(|m| m.content.len() / 4)
            .sum()
    }

    /// Add a message to conversation history
    pub fn push_message(&mut self, role: &str, content: String) {
        self.conversation.push(ConversationMessage {
            role: role.to_string(),
            content,
        });
    }
}

/// Parse a Claude response that starts with a ```json block followed by explanation text
fn parse_step_response(raw: &str) -> (ClaudeStepResponse, String) {
    let trimmed = raw.trim();

    // Look for ```json ... ``` block at the start
    if let Some(json_start) = trimmed.find("```json") {
        let after_fence = &trimmed[json_start + 7..];
        if let Some(json_end) = after_fence.find("```") {
            let json_str = after_fence[..json_end].trim();
            let explanation = after_fence[json_end + 3..].trim().to_string();

            if let Ok(response) = serde_json::from_str::<ClaudeStepResponse>(json_str) {
                return (response, explanation);
            }
        }
    }

    // Fallback: treat entire response as explanation for OVERVIEW
    let fallback = ClaudeStepResponse {
        file: "OVERVIEW".to_string(),
        line_start: 0,
        line_end: 0,
        explanation: String::new(),
        deep_dives: Vec::new(),
        next_file: None,
    };
    (fallback, trimmed.to_string())
}

/// Load tech debt notes from a markdown file
pub fn load_notes(path: &std::path::Path) -> Vec<TechDebtNote> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut notes = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        // Parse lines like: "1. **src/file.rs:10-20** — Note text"
        if let Some(rest) = line.strip_prefix(|c: char| c.is_ascii_digit()) {
            let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.');
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix("**") {
                if let Some(bold_end) = rest.find("**") {
                    let file_part = &rest[..bold_end];
                    let note_text = rest[bold_end + 2..]
                        .trim_start_matches(|c: char| c == ' ' || c == '—' || c == '-')
                        .trim()
                        .to_string();

                    // Parse "file:line_range"
                    let (file, line_range) = if let Some(colon_pos) = file_part.rfind(':') {
                        (
                            file_part[..colon_pos].to_string(),
                            file_part[colon_pos + 1..].to_string(),
                        )
                    } else {
                        (file_part.to_string(), "0-0".to_string())
                    };

                    notes.push(TechDebtNote {
                        file,
                        line_range,
                        note: note_text,
                        timestamp: String::new(),
                    });
                }
            }
        }
    }
    notes
}
