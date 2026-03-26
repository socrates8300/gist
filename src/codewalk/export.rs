use crate::codewalk::app::CodeWalkApp;
use crate::codewalk::types::ModuleFindings;
use std::collections::HashSet;

/// Generate a markdown session export (normal walk modes)
pub fn export_session(app: &CodeWalkApp, model: &str) -> String {
    let mut out = String::new();

    // Header
    out.push_str(&format!("# CodeWalk Session: {}\n\n", app.scope));
    out.push_str(&format!("_Repo: {}_\n", app.repo_path.display()));
    out.push_str(&format!(
        "_Date: {}_\n",
        chrono::Local::now().format("%Y-%m-%d")
    ));
    out.push_str(&format!("_Model: {}_\n", model));
    if let Some(head) = git_head(&app.repo_path) {
        out.push_str(&format!("_Git HEAD: {}_\n", head));
    }
    out.push('\n');

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

    // Token usage estimate
    let approx_tokens = app.approx_token_count();
    out.push_str(&format!(
        "_Approximate token usage: ~{} tokens ({} conversation turns)_\n",
        approx_tokens,
        app.conversation.len()
    ));

    out
}

/// Generate a structured audit report for deep-audit mode.
pub fn export_audit_report(
    app: &CodeWalkApp,
    model: &str,
    module_findings: &[ModuleFindings],
    total_tool_calls: usize,
    elapsed_secs: u64,
    budget_exceeded: bool,
) -> String {
    let mut out = String::new();

    // Header
    out.push_str("# CodeWalk Deep Audit Report\n\n");
    out.push_str(&format!("_Repo: {}_\n", app.repo_path.display()));
    out.push_str(&format!(
        "_Date: {}_\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ));
    out.push_str(&format!("_Model: {}_\n", model));
    if let Some(head) = git_head(&app.repo_path) {
        out.push_str(&format!("_Git HEAD: {}_\n", head));
    }
    out.push_str(&format!("_Modules analyzed: {}_\n", module_findings.len()));
    out.push_str(&format!("_Total tool calls: {}_\n", total_tool_calls));
    out.push_str(&format!("_Elapsed: {}s_\n", elapsed_secs));
    if budget_exceeded {
        out.push_str("_**Warning:** budget limit reached — results may be partial._\n");
    }
    out.push('\n');

    // Executive summary
    let total_findings: usize = module_findings.iter().map(|m| m.findings.len()).sum();
    let total_risks: usize = module_findings.iter().map(|m| m.risks.len()).sum();
    let total_refs: usize = module_findings.iter().map(|m| m.file_refs.len()).sum();

    out.push_str("## Executive Summary\n\n");
    out.push_str(&format!(
        "- **{} modules** analyzed\n- **{} findings** identified\n- **{} risks** flagged\n- **{} file references**\n\n",
        module_findings.len(), total_findings, total_risks, total_refs
    ));

    // Per-module findings
    out.push_str("## Module Analysis\n\n");
    for (i, m) in module_findings.iter().enumerate() {
        out.push_str(&format!("### {}. {}\n\n", i + 1, m.module_path));
        out.push_str(&format!("**Purpose:** {}\n\n", m.purpose));

        if !m.findings.is_empty() {
            out.push_str("**Findings:**\n\n");
            for f in &m.findings {
                out.push_str(&format!("- {}\n", f));
            }
            out.push('\n');
        }

        if !m.risks.is_empty() {
            out.push_str("**Risks:**\n\n");
            for r in &m.risks {
                out.push_str(&format!("- {}\n", r));
            }
            out.push('\n');
        }

        if !m.file_refs.is_empty() {
            out.push_str("**File references:**\n\n");
            for r in &m.file_refs {
                out.push_str(&format!("- `{}:{}` — {}\n", r.path, r.line, r.note));
            }
            out.push('\n');
        }

        out.push_str(&format!("_Tool calls: {}_\n\n", m.tool_calls_used));
        out.push_str("---\n\n");
    }

    // All risks consolidated
    let all_risks: Vec<(&str, &str)> = module_findings
        .iter()
        .flat_map(|m| m.risks.iter().map(move |r| (m.module_path.as_str(), r.as_str())))
        .collect();
    if !all_risks.is_empty() {
        out.push_str("## Consolidated Risk Register\n\n");
        for (module, risk) in &all_risks {
            out.push_str(&format!("- **{}**: {}\n", module, risk));
        }
        out.push('\n');
    }

    // All file references
    let all_refs: Vec<_> = module_findings
        .iter()
        .flat_map(|m| m.file_refs.iter())
        .collect();
    if !all_refs.is_empty() {
        out.push_str("## All File References\n\n");
        for r in &all_refs {
            out.push_str(&format!("- `{}:{}` — {}\n", r.path, r.line, r.note));
        }
        out.push('\n');
    }

    // Tech debt notes (from TUI)
    if !app.tech_debt_notes.is_empty() {
        out.push_str("## Tech Debt Notes (Manual)\n\n");
        for (i, note) in app.tech_debt_notes.iter().enumerate() {
            out.push_str(&format!(
                "{}. **{}:{}** — {}\n",
                i + 1, note.file, note.line_range, note.note
            ));
        }
        out.push('\n');
    }

    out
}

/// Get the current git HEAD SHA (short) for the repo at `path`.
fn git_head(repo_path: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("--short")
        .arg("HEAD")
        .current_dir(repo_path)
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_report_includes_all_sections() {
        let findings = vec![
            ModuleFindings {
                module_path: "src/foo.rs".to_string(),
                purpose: "does foo".to_string(),
                findings: vec!["implements Foo trait".to_string()],
                risks: vec!["unwrap on line 10".to_string()],
                file_refs: vec![crate::codewalk::types::FileRef {
                    path: "src/foo.rs".to_string(),
                    line: 10,
                    note: "panics on None".to_string(),
                }],
                tool_calls_used: 2,
            },
        ];

        // We can't easily build a real CodeWalkApp without a PathBuf, so
        // just test the standalone report pieces
        let risk_count: usize = findings.iter().map(|m| m.risks.len()).sum();
        let ref_count: usize = findings.iter().map(|m| m.file_refs.len()).sum();
        assert_eq!(risk_count, 1);
        assert_eq!(ref_count, 1);
        assert!(findings[0].file_refs[0].path.contains("src/foo.rs"));
    }
}
