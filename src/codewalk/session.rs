//! Phase 3: Session persistence — save, load, list, resume, purge.
//!
//! Sessions are stored as JSON files in ~/.config/gist/sessions/<id>-walk.json.
//! Session IDs are timestamp-slug strings: "20260326-123456-myrepo".

use std::path::{Path, PathBuf};

use crate::codewalk::app::CodeWalkApp;
use crate::codewalk::types::{FullSession, RepoMap, SessionSummary, WalkMode};

// ── Directory helpers ─────────────────────────────────────────────────────────

pub fn sessions_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gist")
        .join("sessions")
}

fn ensure_sessions_dir() -> PathBuf {
    let dir = sessions_dir();
    let _ = std::fs::create_dir_all(&dir);
    dir
}

// ── Save ──────────────────────────────────────────────────────────────────────

/// Serialize the current app state to a session file.
/// Returns the session ID (also stored in `app.session_id`).
pub fn save_full_session(
    app: &CodeWalkApp,
    model: &str,
    mode: &WalkMode,
    repo_map: Option<&RepoMap>,
) -> Result<String, Box<dyn std::error::Error>> {
    let dir = ensure_sessions_dir();

    // Reuse existing ID if this is a continuation
    let id = app.session_id.clone().unwrap_or_else(|| {
        let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let slug = app
            .repo_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repo".to_string());
        format!("{ts}-{slug}")
    });

    let session = FullSession {
        id: id.clone(),
        repo_path: app.repo_path.to_string_lossy().into_owned(),
        model: model.to_string(),
        mode: mode.clone(),
        started_at: app.started_at.clone(),
        last_updated: chrono::Local::now().to_rfc3339(),
        steps: app.steps.clone(),
        conversation: app.conversation.clone(),
        tech_debt_notes: app.tech_debt_notes.clone(),
        repo_map: repo_map.cloned(),
    };

    let filename = format!("{id}-walk.json");
    let json = serde_json::to_string_pretty(&session)?;
    std::fs::write(dir.join(&filename), &json)?;
    Ok(id)
}

// ── Load ──────────────────────────────────────────────────────────────────────

pub fn load_session(session_id: &str) -> Result<FullSession, Box<dyn std::error::Error>> {
    let dir = sessions_dir();
    let filename = format!("{session_id}-walk.json");
    let path = dir.join(&filename);
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read session '{session_id}': {e}"))?;
    let session: FullSession = serde_json::from_str(&content)?;
    Ok(session)
}

// ── List ──────────────────────────────────────────────────────────────────────

/// Return all sessions sorted by start time (newest first).
pub fn list_sessions() -> Vec<SessionSummary> {
    let dir = sessions_dir();
    let mut summaries = Vec::new();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return summaries;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if !filename.ends_with("-walk.json") {
            continue;
        }

        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(session) = serde_json::from_str::<FullSession>(&content) else {
            continue;
        };

        summaries.push(SessionSummary {
            id: session.id,
            repo_path: session.repo_path,
            model: session.model,
            mode: session.mode,
            started_at: session.started_at,
            step_count: session.steps.len(),
            filename,
        });
    }

    summaries.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    summaries
}

// ── Purge ─────────────────────────────────────────────────────────────────────

/// Delete all session files. Returns the count deleted.
pub fn purge_sessions() -> usize {
    let dir = sessions_dir();
    let mut count = 0;
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if std::fs::remove_file(&path).is_ok() {
                count += 1;
            }
        }
    }
    count
}

/// Delete sessions older than `retention_days`.
pub fn purge_old_sessions(retention_days: u32) {
    let dir = sessions_dir();
    let cutoff = chrono::Local::now()
        - chrono::Duration::try_days(retention_days as i64).unwrap_or_default();

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(session) = serde_json::from_str::<FullSession>(&content) else {
            continue;
        };
        if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(&session.last_updated) {
            if dt < cutoff {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

// ── Memory / prior sessions ───────────────────────────────────────────────────

/// Return prior sessions for the same repo path (for memory injection).
pub fn find_prior_sessions(repo_path: &Path) -> Vec<SessionSummary> {
    let repo_str = repo_path.to_string_lossy();
    list_sessions()
        .into_iter()
        .filter(|s| Path::new(&s.repo_path) == repo_path || s.repo_path == repo_str.as_ref())
        .collect()
}

/// Build a brief memory note from prior sessions for injection into the system context.
pub fn build_memory_note(prior: &[SessionSummary]) -> String {
    if prior.is_empty() {
        return String::new();
    }
    let mut note = String::from(
        "\n\n---\n**Prior walkthroughs on this repository (from memory):**\n",
    );
    for s in prior.iter().take(3) {
        note.push_str(&format!(
            "- {} — {} steps, mode: {}, model: {}\n",
            s.started_at, s.step_count, s.mode, s.model
        ));
    }
    note.push_str(
        "\nUse this context to avoid repeating covered ground and to build on prior analysis.",
    );
    note
}

// ── Compaction ────────────────────────────────────────────────────────────────

/// Trim oldest conversation turns to stay under the token threshold.
/// Always preserves the first message (initial context) and the last 4 messages.
pub fn compact_conversation(
    conversation: &mut Vec<crate::codewalk::types::ConversationMessage>,
    threshold: usize,
) -> bool {
    let approx_tokens: usize = conversation.iter().map(|m| m.content.len() / 4).sum();
    if approx_tokens <= threshold {
        return false;
    }

    // Keep first message (initial context) + last 4 messages
    if conversation.len() <= 5 {
        return false;
    }

    // Remove messages 1..N-4 (keep index 0 and tail)
    let tail_start = conversation.len().saturating_sub(4);
    let to_remove = 1..tail_start;
    conversation.drain(to_remove);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codewalk::types::ConversationMessage;

    #[test]
    fn compaction_leaves_first_and_tail() {
        let mut conv: Vec<ConversationMessage> = (0..10)
            .map(|i| ConversationMessage {
                role: "user".to_string(),
                content: "x".repeat(1000 * (i + 1)), // escalating size
            })
            .collect();
        let compacted = compact_conversation(&mut conv, 100); // threshold very low
        assert!(compacted);
        assert_eq!(conv[0].content.len(), 1000); // first preserved
        assert_eq!(conv.last().unwrap().content.len(), 10000); // last preserved
        assert!(conv.len() <= 5);
    }

    #[test]
    fn compaction_noop_under_threshold() {
        let mut conv = vec![
            ConversationMessage { role: "user".to_string(), content: "short".to_string() },
        ];
        let compacted = compact_conversation(&mut conv, 10000);
        assert!(!compacted);
        assert_eq!(conv.len(), 1);
    }

    #[test]
    fn memory_note_empty_for_no_prior() {
        assert!(build_memory_note(&[]).is_empty());
    }
}
