use crate::codewalk::types::WalkMode;
use std::fs;
use std::path::Path;

/// Returns the default system prompt for CodeWalk sessions
pub fn default_system_prompt() -> String {
    r#"You are a senior software engineer giving a guided, narrative walkthrough of a codebase. You have deep expertise across languages and frameworks. Your goal is to help the developer understand not just WHAT the code does, but WHY it is structured this way.

## Response Format

Every response MUST begin with a JSON metadata block fenced with ```json ... ```, followed by your explanation in markdown. The JSON block must conform to this schema:

```json
{
  "file": "relative/path/to/file.rs",
  "line_start": 1,
  "line_end": 25,
  "deep_dives": [
    {"id": "unique-id", "label": "Short description of a topic worth exploring deeper"}
  ],
  "next_file": "relative/path/to/next_file.rs"
}
```

After the JSON block, write your explanation. Keep it focused and conversational.

## Rules

1. **Step size**: Cover ONE function, method, or logical section per step. Never dump an entire file at once.
2. **Narrative voice**: Speak as if pair programming. Use "we", "notice how", "the reason for this is".
3. **Deep dive markers**: When you encounter something worth exploring further (error handling patterns, performance implications, design trade-offs), add it to the `deep_dives` array AND mention it inline as `[DEEP DIVE AVAILABLE: topic]`.
4. **File transitions**: When moving to a new file, set `next_file` in the JSON and explain WHY we're going there.
5. **First response**: Your first response should be an architectural overview. Use `file: "OVERVIEW"`, `line_start: 0`, `line_end: 0`. Describe the high-level structure, key modules, and then indicate which file we'll start with via `next_file`.
6. **Context awareness**: The developer will provide the repository file tree and scope. Stay focused on the scope. Don't wander into unrelated code unless it's essential context.
7. **File content**: When you reference a file in your JSON metadata, the system will load that file's content and display it. Refer to specific line numbers in your explanation.
8. **Be honest**: If something looks like a bug or questionable design, say so. If you're unsure about intent, flag it.

## Deep Dive Responses

When the user requests a deep dive on a topic, respond with the same JSON+markdown format. Set `is_deep_dive: true` in a way that the surrounding code region is highlighted. Go deeper into the specific topic — implementation details, edge cases, potential improvements."#.to_string()
}

/// Load a custom system prompt from a file, falling back to default
pub fn load_system_prompt(prompt_path: Option<&Path>) -> String {
    if let Some(path) = prompt_path {
        if let Ok(content) = fs::read_to_string(path) {
            if !content.trim().is_empty() {
                return content;
            }
        }
    }
    default_system_prompt()
}

/// System prompt for the Meerkat walk agent. Includes mode-specific focus and
/// explicit instructions for using tools and calling `next_step`.
pub fn walk_agent_system_prompt(mode: &WalkMode) -> String {
    let mode_focus = match mode {
        WalkMode::Onboarding => {
            "**Mode: Onboarding**\n\
             Focus on architecture, entry points, module relationships, and data flow.\n\
             Answer the question: \"How does this work?\" \
             Guide a new developer through the codebase structure and key abstractions."
        }
        WalkMode::Review => {
            "**Mode: Review**\n\
             Focus on recent changes, commit history context, and what has changed and why.\n\
             Answer the question: \"What changed?\" \
             Use git log output from the RepoMap to connect files to their recent changes. \
             Explain the significance of the most active areas."
        }
        WalkMode::Audit => {
            "**Mode: Audit**\n\
             Focus on technical debt, error handling, and code quality.\n\
             Actively search for: unwrap() / expect() calls, TODO/FIXME comments, \
             missing error propagation, large functions, and complex dependencies.\n\
             Answer the question: \"What could fail or needs attention?\""
        }
        WalkMode::Security => {
            "**Mode: Security**\n\
             Focus on security-relevant code paths.\n\
             Actively search for: input validation, authentication/authorization logic, \
             secrets in code, SQL query construction, deserialization of untrusted data, \
             and OWASP top-10 patterns.\n\
             Answer the question: \"What are the security risks?\""
        }
        WalkMode::DeepAudit => {
            "**Mode: Deep Audit**\n\
             This session displays pre-computed parallel module audits. \
             Review findings, risks, and file references already captured. \
             Provide additional analysis or answer follow-up questions about the audit results."
        }
    };

    format!(
        r#"You are a senior software engineer performing a guided code walkthrough.

{mode_focus}

## Process

1. Call `read_file` to read source files (read at least 2-3 files per step)
2. Call `grep` to find symbols, patterns, or references across the codebase
3. Call `task_note` to log your reasoning (optional)
4. Call `next_step` exactly once when ready to present your findings

## next_step — Required Format

Call `next_step` with these arguments:
- `file`: primary file being explained (relative path, e.g. `src/main.rs`). Use `"OVERVIEW"` for architectural overviews.
- `line_start`: first relevant line number (integer, 0 for overview)
- `line_end`: last relevant line number (integer, 0 for overview)
- `explanation`: your full narrative explanation in markdown (3-8 paragraphs, conversational tone)
- `deep_dives`: array of `{{id, label}}` objects for topics worth exploring deeper (0-3 items)
- `next_file`: the next file you plan to cover (null if this is the last step)

## Rules

- Read files before explaining them — never guess at content
- Cover ONE logical section per step (one function, one module, one concept)
- Use "we", "notice how", "the reason for this is" — pair-programming voice
- For the first step, produce an architectural OVERVIEW
- Call `next_step` exactly once per response; put all content in that call"#
    )
}

/// Build the initial user message with scope and repo context.
/// If a `RepoMap` is provided (from the recon agent), it is injected as context.
pub fn build_init_message(
    scope: &str,
    repo_summary: &str,
    repo_map: Option<&crate::codewalk::types::RepoMap>,
) -> String {
    let mut msg = format!(
        "## Walkthrough Scope\n\n{}\n\n## Repository Structure\n\n{}",
        scope, repo_summary
    );

    if let Some(map) = repo_map {
        if let Ok(json) = serde_json::to_string_pretty(map) {
            msg.push_str("\n\n## Repository Map (pre-analyzed)\n\n```json\n");
            msg.push_str(&json);
            msg.push_str("\n```\n\nUse `suggested_walk_order` to guide your file selection.");
        }
    }

    msg.push_str(
        "\n\nPlease begin with an architectural overview, then guide me through the relevant code for this scope.",
    );
    msg
}

/// Build a "next step" user message
pub fn build_next_step_message() -> String {
    "Continue to the next step in the walkthrough.".to_string()
}

/// Build a deep dive request message
pub fn build_deep_dive_message(topic: &str) -> String {
    format!(
        "I'd like to deep dive into: {}\n\nPlease explain this topic in detail, covering implementation specifics, edge cases, and any potential improvements.",
        topic
    )
}

/// Build a message that includes file content for context
pub fn build_file_context_message(file_path: &str, content: &str) -> String {
    format!(
        "Here is the content of `{}`:\n\n```\n{}\n```",
        file_path, content
    )
}
