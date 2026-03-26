use crate::codewalk::app::CodeWalkApp;
use std::collections::HashSet;

/// Generate a markdown session export
pub fn export_session(app: &CodeWalkApp, model: &str) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!("# CodeWalk Session: {}\n\n", app.scope));
    out.push_str(&format!("_Repo: {}_\n", app.repo_path.display()));
    out.push_str(&format!(
        "_Date: {}_\n",
        chrono::Local::now().format("%Y-%m-%d")
    ));
    out.push_str(&format!("_Model: {}_\n\n", model));

    // Architectural overview
    if !app.overview_text.is_empty() {
        out.push_str("## Architectural Overview\n\n");
        out.push_str(&app.overview_text);
        out.push_str("\n\n");
    }

    // Files walked
    let mut seen_files = HashSet::new();
    let mut files_list = Vec::new();
    for step in &app.steps {
        let file = &step.response.file;
        if file != "OVERVIEW" && seen_files.insert(file.clone()) {
            files_list.push(file.clone());
        }
    }

    if !files_list.is_empty() {
        out.push_str("## Files Walked\n\n");
        for (i, file) in files_list.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", i + 1, file));
        }
        out.push('\n');
    }

    // Deep dives taken
    if !app.all_deep_dives.is_empty() {
        out.push_str("## Deep Dives Taken\n\n");
        for (step_idx, dd) in &app.all_deep_dives {
            out.push_str(&format!("- {} (step {})\n", dd.label, step_idx + 1));
        }
        out.push('\n');
    }

    // Tech debt notes
    if !app.tech_debt_notes.is_empty() {
        out.push_str("## Tech Debt Notes\n\n");
        for (i, note) in app.tech_debt_notes.iter().enumerate() {
            out.push_str(&format!(
                "{}. **{}:{}** — {}\n",
                i + 1,
                note.file,
                note.line_range,
                note.note
            ));
        }
        out.push('\n');
    }

    out
}
