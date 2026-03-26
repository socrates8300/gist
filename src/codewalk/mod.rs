pub mod app;
pub mod claude;
pub mod export;
#[cfg(feature = "meerkat")]
pub mod meerkat_spike;
#[cfg(feature = "meerkat")]
pub mod agent;
#[cfg(feature = "meerkat")]
pub mod recon;
#[cfg(feature = "meerkat")]
pub mod deep_audit;
pub mod prompt;
pub mod repo;
pub mod session;
pub mod types;
pub mod ui;

use crate::config::Config;
use app::{CWInputMode, CWPanel, CodeWalkApp};
use claude::{resolve_api_config, spawn_stream_request};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use prompt::{build_deep_dive_message, build_file_context_message, build_init_message, build_next_step_message, load_system_prompt};
use ratatui::{backend::CrosstermBackend, Terminal};
use repo::RepoIndex;
use std::{error::Error, io, path::PathBuf, time::Duration};
use tokio::sync::mpsc;
use types::{StreamEvent, WalkMode};

/// Main entry point for a CodeWalk session
pub async fn run_codewalk(
    scope: String,
    repo_path: PathBuf,
    model: Option<String>,
    prompt_file: Option<PathBuf>,
    notes_file: Option<PathBuf>,
    output_path: Option<PathBuf>,
    config: Config,
    no_meerkat: bool,
    mode: WalkMode,
    resume_id: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let model = model.unwrap_or_else(|| config.ai_model.clone().unwrap_or_else(|| "z-ai/glm-5-turbo".to_string()));

    // Resolve API config
    let api_config = resolve_api_config(
        &model,
        config.anthropic_api_key.as_deref(),
        config.tag_api_key.as_deref(),
        config.ai_base_url.as_deref(),
    )
    .map_err(|e| -> Box<dyn Error> { e.into() })?;

    // Index repository
    eprintln!("Indexing repository...");
    let mut repo_index = RepoIndex::build(&repo_path)?;
    let repo_summary = repo_index.summary();
    eprintln!(
        "Found {} files across {} languages",
        repo_index.file_count,
        repo_index.languages.len()
    );

    // Capture model string for session log (already resolved to String above)
    let model_str = model.clone();

    // Run recon agent to build a RepoMap before entering the TUI
    #[cfg(feature = "meerkat")]
    let repo_map: Option<types::RepoMap> = if !no_meerkat {
        eprintln!("Mapping repository...");
        let cw_cfg = config.codewalk.as_ref().cloned().unwrap_or_default();
        match recon::run_recon(
            &api_config,
            &repo_path,
            4000,
            cw_cfg.recon_max_tool_calls,
            cw_cfg.recon_max_wall_seconds,
        ).await {
            Ok(map) => {
                recon::save_recon_log(&map, &repo_path);
                eprintln!(
                    "Repository mapped ({} modules, {} entry points)",
                    map.key_modules.len(),
                    map.entry_points.len()
                );
                Some(map)
            }
            Err(e) => {
                eprintln!("Warning: recon failed ({e}), continuing without repo map.");
                None
            }
        }
    } else {
        None
    };
    #[cfg(not(feature = "meerkat"))]
    let repo_map: Option<types::RepoMap> = None;

    // Load system prompt — mode-specific when using the walk agent
    #[cfg(feature = "meerkat")]
    let system_prompt = if !no_meerkat && prompt_file.is_none() {
        prompt::walk_agent_system_prompt(&mode)
    } else {
        load_system_prompt(prompt_file.as_deref())
    };
    #[cfg(not(feature = "meerkat"))]
    let system_prompt = load_system_prompt(prompt_file.as_deref());

    // Phase 4: deep-audit — run all sub-agents before entering TUI
    #[cfg(feature = "meerkat")]
    let (deep_audit_findings, deep_audit_budget): (
        Vec<types::ModuleFindings>,
        Option<std::sync::Arc<deep_audit::BudgetState>>,
    ) = if mode == types::WalkMode::DeepAudit && !no_meerkat {
        let cw_cfg = config.codewalk.as_ref();
        let budget_config = types::BudgetConfig {
            max_tokens: cw_cfg.map(|c| c.max_tokens).unwrap_or(100_000),
            max_tool_calls: cw_cfg.map(|c| c.max_tool_calls).unwrap_or(200),
            max_wall_seconds: cw_cfg.map(|c| c.max_wall_seconds).unwrap_or(300),
            max_subagents: cw_cfg.map(|c| c.max_subagents).unwrap_or(4),
        };
        let modules = repo_map
            .as_ref()
            .map(|m| m.key_modules.clone())
            .unwrap_or_default();
        if modules.is_empty() {
            eprintln!(
                "Warning: no modules in repo map. Recon may have failed; try without --no-meerkat."
            );
            (vec![], None)
        } else {
            eprintln!(
                "Running deep audit: {} modules, max {} concurrent, {} tool call budget...",
                modules.len(),
                budget_config.max_subagents,
                budget_config.max_tool_calls,
            );
            let budget = deep_audit::BudgetState::new(budget_config);
            let findings = deep_audit::run_deep_audit(
                &api_config,
                modules,
                &repo_path,
                std::sync::Arc::clone(&budget),
            )
            .await;
            let tool_calls = budget
                .total_tool_calls
                .load(std::sync::atomic::Ordering::SeqCst);
            let exceeded = budget
                .budget_exceeded
                .load(std::sync::atomic::Ordering::SeqCst);
            eprintln!(
                "Deep audit complete: {} modules, {} tool calls{}",
                findings.len(),
                tool_calls,
                if exceeded { " (budget limit reached)" } else { "" }
            );
            (findings, Some(budget))
        }
    } else {
        (vec![], None)
    };

    // Phase 3: compaction threshold from config
    let compaction_threshold = config
        .codewalk
        .as_ref()
        .map(|c| c.compaction_threshold)
        .unwrap_or(50_000);

    // Phase 3: retention — purge sessions older than configured threshold
    {
        let retention_days = config
            .codewalk
            .as_ref()
            .map(|c| c.session_retention_days)
            .unwrap_or(30);
        session::purge_old_sessions(retention_days);
    }

    // Initialize app state — either fresh or resumed
    let mut app = if let Some(ref id) = resume_id {
        match session::load_session(id) {
            Ok(saved) => {
                eprintln!("Resuming session '{}' ({} steps)", id, saved.steps.len());
                let mut a = CodeWalkApp::from_session(saved, output_path);
                a.scope = scope.clone();
                a
            }
            Err(e) => {
                eprintln!("Warning: cannot load session '{id}': {e}. Starting fresh.");
                CodeWalkApp::new(scope.clone(), repo_path.clone(), output_path)
            }
        }
    } else {
        CodeWalkApp::new(scope.clone(), repo_path.clone(), output_path)
    };

    app.compaction_threshold = compaction_threshold;

    // Configure walk agent settings
    #[cfg(feature = "meerkat")]
    {
        app.use_meerkat = !no_meerkat;
        app.walk_mode = mode.clone();
    }

    // Load existing notes if provided
    if let Some(notes_path) = &notes_file {
        app.tech_debt_notes = app::load_notes(notes_path);
        if !app.tech_debt_notes.is_empty() {
            app.tech_debt_visible = true;
        }
    }

    // Phase 4: pre-load deep-audit findings as WalkSteps before TUI starts
    #[cfg(feature = "meerkat")]
    if !deep_audit_findings.is_empty() {
        for (i, finding) in deep_audit_findings.iter().enumerate() {
            let explanation = deep_audit::format_findings_as_explanation(finding);
            let response = types::ClaudeStepResponse {
                file: finding.module_path.clone(),
                line_start: 0,
                line_end: 0,
                explanation,
                deep_dives: vec![],
                next_file: None,
            };
            let file_content = repo_index.read_file(&finding.module_path).unwrap_or_default();
            app.steps.push(types::WalkStep {
                index: i,
                response,
                file_content,
                is_deep_dive: false,
                parent_step: None,
            });
        }
        if !app.steps.is_empty() {
            app.current_step = 0;
            app.mode = app::CWInputMode::Normal;
        }
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create streaming channel
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<StreamEvent>();

    // Skip initial streaming if steps already loaded (resume or deep-audit)
    let is_deep_audit_preloaded = {
        #[cfg(feature = "meerkat")]
        { mode == types::WalkMode::DeepAudit && !app.steps.is_empty() }
        #[cfg(not(feature = "meerkat"))]
        { false }
    };
    if (resume_id.is_some() || is_deep_audit_preloaded) && !app.steps.is_empty() {
        let status = if is_deep_audit_preloaded {
            let exceeded = {
                #[cfg(feature = "meerkat")]
                {
                    deep_audit_budget
                        .as_ref()
                        .map(|b| b.budget_exceeded.load(std::sync::atomic::Ordering::SeqCst))
                        .unwrap_or(false)
                }
                #[cfg(not(feature = "meerkat"))]
                { false }
            };
            format!(
                "Deep audit: {} modules ready{}. Use n/p to navigate.",
                app.steps.len(),
                if exceeded { " (budget limit reached)" } else { "" }
            )
        } else {
            format!("Resumed at step {}", app.current_step + 1)
        };
        app.set_status(status);
    } else {
        // Phase 3: inject memory from prior sessions on this repo
        let prior = session::find_prior_sessions(&repo_path);
        let enable_memory = config
            .codewalk
            .as_ref()
            .map(|c| c.enable_memory)
            .unwrap_or(true);
        let memory_note = if enable_memory && !prior.is_empty() {
            session::build_memory_note(&prior)
        } else {
            String::new()
        };

        // Send initial message to Claude
        let mut init_message = build_init_message(&scope, &repo_summary, repo_map.as_ref());
        if !memory_note.is_empty() {
            init_message.push_str(&memory_note);
        }
        app.push_message("user", init_message.clone());
        app.start_streaming();
        app.set_status("Requesting architectural overview...".to_string());

        #[cfg(feature = "meerkat")]
        if app.use_meerkat {
            agent::spawn_walk_step(
                api_config.clone(),
                system_prompt.clone(),
                app.conversation.clone(),
                repo_path.clone(),
                stream_tx.clone(),
                0,
                None,
            );
        } else {
            spawn_stream_request(
                api_config.clone(),
                system_prompt.clone(),
                app.conversation.clone(),
                stream_tx.clone(),
            );
        }
        #[cfg(not(feature = "meerkat"))]
        spawn_stream_request(
            api_config.clone(),
            system_prompt.clone(),
            app.conversation.clone(),
            stream_tx.clone(),
        );
    }

    // Main event loop
    let result = run_event_loop(
        &mut terminal,
        &mut app,
        &mut stream_rx,
        &stream_tx,
        &api_config,
        &system_prompt,
        &mut repo_index,
    )
    .await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    // Export session if output path is set
    if let Some(output_path) = &app.output_path {
        #[cfg(feature = "meerkat")]
        let summary = if !deep_audit_findings.is_empty() {
            let (tool_calls, elapsed, exceeded) = deep_audit_budget
                .as_ref()
                .map(|b| {
                    (
                        b.total_tool_calls.load(std::sync::atomic::Ordering::SeqCst),
                        b.elapsed_secs(),
                        b.budget_exceeded.load(std::sync::atomic::Ordering::SeqCst),
                    )
                })
                .unwrap_or((0, 0, false));
            export::export_audit_report(
                &app,
                &model_str,
                &deep_audit_findings,
                tool_calls,
                elapsed,
                exceeded,
            )
        } else {
            export::export_session(&app, &model_str)
        };
        #[cfg(not(feature = "meerkat"))]
        let summary = export::export_session(&app, &model_str);
        std::fs::write(output_path, &summary)?;
        eprintln!("Session exported to {}", output_path.display());
    }

    // Auto-save full session to ~/.config/gist/sessions/
    if !app.steps.is_empty() {
        let walk_mode = {
            #[cfg(feature = "meerkat")]
            { app.walk_mode.clone() }
            #[cfg(not(feature = "meerkat"))]
            { types::WalkMode::default() }
        };
        match session::save_full_session(&app, &model_str, &walk_mode, repo_map.as_ref()) {
            Ok(id) => eprintln!("Session saved: {id}"),
            Err(e) => eprintln!("Warning: could not save session: {e}"),
        }
    }

    result
}

async fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut CodeWalkApp,
    stream_rx: &mut mpsc::UnboundedReceiver<StreamEvent>,
    stream_tx: &mpsc::UnboundedSender<StreamEvent>,
    api_config: &types::ApiConfig,
    system_prompt: &str,
    repo_index: &mut RepoIndex,
) -> Result<(), Box<dyn Error>> {
    loop {
        // 1. Collect streaming tokens
        while let Ok(event) = stream_rx.try_recv() {
            match event {
                StreamEvent::Token(text) => {
                    app.streaming_text.push_str(&text);
                }
                StreamEvent::Done => {
                    finalize_current_step(app, repo_index);
                    // Phase 3: auto-compaction
                    if session::compact_conversation(
                        &mut app.conversation,
                        app.compaction_threshold,
                    ) {
                        app.set_status("Context compacted to stay within token budget.".to_string());
                    }
                }
                StreamEvent::Error(e) => {
                    app.is_streaming = false;
                    app.mode = CWInputMode::Normal;
                    app.set_status(format!("Error: {}", e));
                }
            }
        }

        // 2. Render
        terminal.draw(|f| ui::render_codewalk(f, app))?;

        // 3. Handle keyboard input
        if crossterm::event::poll(Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                handle_key_input(
                    app,
                    key.code,
                    key.modifiers,
                    stream_tx,
                    api_config,
                    system_prompt,
                    repo_index,
                );
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

fn finalize_current_step(app: &mut CodeWalkApp, repo_index: &mut RepoIndex) {
    // Try to determine what file this step references by peeking at the streamed JSON
    let raw = app.streaming_text.clone();
    let file_content = extract_file_and_load(&raw, repo_index);

    // Save assistant message to conversation
    app.push_message("assistant", raw.clone());

    app.finalize_step(file_content);
    app.set_status(format!("Step {} loaded", app.steps.len()));
}

fn extract_file_and_load(raw: &str, repo_index: &mut RepoIndex) -> String {
    // Quick parse to find the file field from the JSON envelope
    if let Some(json_start) = raw.find("```json") {
        let after_fence = &raw[json_start + 7..];
        if let Some(json_end) = after_fence.find("```") {
            let json_str = after_fence[..json_end].trim();
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(file) = parsed.get("file").and_then(|f| f.as_str()) {
                    if file != "OVERVIEW" {
                        if let Ok(content) = repo_index.read_file(file) {
                            return content;
                        }
                    }
                }
            }
        }
    }
    String::new()
}

fn request_next_step(
    app: &mut CodeWalkApp,
    stream_tx: &mpsc::UnboundedSender<StreamEvent>,
    api_config: &types::ApiConfig,
    system_prompt: &str,
    repo_index: &mut RepoIndex,
) {
    // If the current step has a next_file hint, provide its content
    let mut user_msg = build_next_step_message();

    if let Some(step) = app.current_step_data() {
        if let Some(next_file) = &step.response.next_file {
            if let Ok(content) = repo_index.read_file(next_file) {
                user_msg = format!(
                    "{}\n\n{}",
                    user_msg,
                    build_file_context_message(next_file, &content)
                );
            }
        }
    }

    app.push_message("user", user_msg);
    app.start_streaming();
    app.set_status("Requesting next step...".to_string());

    #[cfg(feature = "meerkat")]
    if app.use_meerkat {
        let next_hint = app.current_step_data()
            .and_then(|s| s.response.next_file.clone());
        let step_number = app.steps.len();
        agent::spawn_walk_step(
            api_config.clone(),
            system_prompt.to_string(),
            app.conversation.clone(),
            app.repo_path.clone(),
            stream_tx.clone(),
            step_number,
            next_hint,
        );
        return;
    }

    spawn_stream_request(
        api_config.clone(),
        system_prompt.to_string(),
        app.conversation.clone(),
        stream_tx.clone(),
    );
}

fn request_deep_dive(
    app: &mut CodeWalkApp,
    topic: &str,
    stream_tx: &mpsc::UnboundedSender<StreamEvent>,
    api_config: &types::ApiConfig,
    system_prompt: &str,
) {
    let msg = build_deep_dive_message(topic);
    app.push_message("user", msg);
    app.start_streaming();
    app.set_status(format!("Deep diving: {}...", topic));

    spawn_stream_request(
        api_config.clone(),
        system_prompt.to_string(),
        app.conversation.clone(),
        stream_tx.clone(),
    );
}

fn handle_key_input(
    app: &mut CodeWalkApp,
    code: KeyCode,
    modifiers: KeyModifiers,
    stream_tx: &mpsc::UnboundedSender<StreamEvent>,
    api_config: &types::ApiConfig,
    system_prompt: &str,
    repo_index: &mut RepoIndex,
) {
    // Ctrl-C exits gracefully from any mode
    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
        app.should_quit = true;
        return;
    }

    match app.mode.clone() {
        CWInputMode::Normal => {
            handle_normal_mode(app, code, modifiers, stream_tx, api_config, system_prompt, repo_index);
        }
        CWInputMode::WaitingForStep => {
            // Only allow quit while streaming
            if code == KeyCode::Char('q') {
                app.mode = CWInputMode::ConfirmQuit;
            }
        }
        CWInputMode::NoteInput => {
            handle_note_input(app, code, modifiers);
        }
        CWInputMode::SearchInFile => {
            handle_search_input(app, code);
        }
        CWInputMode::Help => {
            if code == KeyCode::Esc || code == KeyCode::Char('?') {
                app.mode = CWInputMode::Normal;
            }
        }
        CWInputMode::DeepDiveList => {
            handle_deep_dive_list(app, code, stream_tx, api_config, system_prompt);
        }
        CWInputMode::ConfirmQuit => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    app.should_quit = true;
                }
                KeyCode::Esc | KeyCode::Char('n') => {
                    app.mode = if app.is_streaming {
                        CWInputMode::WaitingForStep
                    } else {
                        CWInputMode::Normal
                    };
                }
                _ => {}
            }
        }
    }
}

fn handle_normal_mode(
    app: &mut CodeWalkApp,
    code: KeyCode,
    modifiers: KeyModifiers,
    stream_tx: &mpsc::UnboundedSender<StreamEvent>,
    api_config: &types::ApiConfig,
    system_prompt: &str,
    repo_index: &mut RepoIndex,
) {
    // Handle pending 'g' for 'gg' command
    if app.pending_g {
        app.pending_g = false;
        if code == KeyCode::Char('g') {
            app.go_start();
            return;
        }
        // If not 'g', fall through to normal handling
    }

    match code {
        // Step navigation
        KeyCode::Char('n') => {
            if app.go_next() {
                request_next_step(app, stream_tx, api_config, system_prompt, repo_index);
            }
        }
        KeyCode::Char('p') => {
            app.go_prev();
        }
        KeyCode::Char('N') => {
            let needs_new = app.jump_forward(5);
            if needs_new {
                request_next_step(app, stream_tx, api_config, system_prompt, repo_index);
            }
        }
        KeyCode::Char('P') => {
            app.jump_back(5);
        }
        KeyCode::Char('g') => {
            app.pending_g = true;
        }
        KeyCode::Char('G') => {
            app.go_end();
        }

        // Scrolling
        KeyCode::Char('j') | KeyCode::Down => {
            match app.focused_panel {
                CWPanel::Code => { app.code_scroll = app.code_scroll.saturating_add(1); }
                CWPanel::Explanation => { app.explanation_scroll = app.explanation_scroll.saturating_add(1); }
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            match app.focused_panel {
                CWPanel::Code => { app.code_scroll = app.code_scroll.saturating_sub(1); }
                CWPanel::Explanation => { app.explanation_scroll = app.explanation_scroll.saturating_sub(1); }
            }
        }
        KeyCode::Char('J') => {
            app.code_scroll = app.code_scroll.saturating_add(1);
        }
        KeyCode::Char('K') => {
            app.code_scroll = app.code_scroll.saturating_sub(1);
        }
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            match app.focused_panel {
                CWPanel::Code => {
                    app.code_scroll = app.code_scroll.saturating_add(15);
                }
                CWPanel::Explanation => {
                    app.explanation_scroll = app.explanation_scroll.saturating_add(15);
                }
            }
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            match app.focused_panel {
                CWPanel::Code => {
                    app.code_scroll = app.code_scroll.saturating_sub(15);
                }
                CWPanel::Explanation => {
                    app.explanation_scroll = app.explanation_scroll.saturating_sub(15);
                }
            }
        }
        KeyCode::Tab => {
            app.toggle_panel();
        }

        // Deep dive
        KeyCode::Char('d') => {
            let dives = app.current_deep_dives();
            if let Some(first) = dives.first() {
                let label = first.label.clone();
                request_deep_dive(app, &label, stream_tx, api_config, system_prompt);
            } else {
                app.set_status("No deep dive topics available at this step".to_string());
            }
        }
        KeyCode::Char('D') => {
            if app.all_deep_dives.is_empty() {
                app.set_status("No deep dive topics discovered yet".to_string());
            } else {
                app.deep_dive_cursor = 0;
                app.mode = CWInputMode::DeepDiveList;
            }
        }

        // Tech debt
        KeyCode::Char('t') => {
            if let Some((start, end)) = app.highlight_range() {
                let lines: Vec<&str> = app.current_code().lines().collect();
                let lo = start.saturating_sub(1);
                let hi = (end.saturating_sub(1)).min(lines.len().saturating_sub(1));
                let snippet = lines[lo..=hi].join("\n");
                app.note_input_buffer = format!("```\n{snippet}\n```\n");
            } else {
                app.note_input_buffer.clear();
            }
            app.mode = CWInputMode::NoteInput;
        }
        KeyCode::Char('y') => {
            use clipboard::ClipboardProvider;
            if let Some((start, end)) = app.highlight_range() {
                let lines: Vec<&str> = app.current_code().lines().collect();
                let lo = start.saturating_sub(1);
                let hi = (end.saturating_sub(1)).min(lines.len().saturating_sub(1));
                let snippet = lines[lo..=hi].join("\n");
                match clipboard::ClipboardContext::new().and_then(|mut ctx| ctx.set_contents(snippet)) {
                    Ok(_) => app.set_status(format!("Yanked lines {}–{} to clipboard", start, end)),
                    Err(e) => app.set_status(format!("Clipboard error: {e}")),
                }
            } else {
                app.set_status("No highlighted range to yank".to_string());
            }
        }
        KeyCode::Char('T') => {
            app.tech_debt_visible = !app.tech_debt_visible;
        }

        // Search
        KeyCode::Char('s') => {
            app.search_query.clear();
            app.mode = CWInputMode::SearchInFile;
        }

        // Help
        KeyCode::Char('?') => {
            app.mode = CWInputMode::Help;
        }

        // Quit
        KeyCode::Char('q') => {
            app.mode = CWInputMode::ConfirmQuit;
        }

        _ => {}
    }
}

fn handle_note_input(app: &mut CodeWalkApp, code: KeyCode, modifiers: KeyModifiers) {
    match code {
        KeyCode::Enter => {
            let note = app.note_input_buffer.clone();
            if !note.trim().is_empty() {
                app.add_tech_debt_note(note);
                app.tech_debt_visible = true;
                app.set_status("Tech debt note added".to_string());
            }
            app.note_input_buffer.clear();
            app.mode = CWInputMode::Normal;
        }
        KeyCode::Esc => {
            app.note_input_buffer.clear();
            app.mode = CWInputMode::Normal;
        }
        KeyCode::Backspace => {
            app.note_input_buffer.pop();
        }
        KeyCode::Char('v') if modifiers.contains(KeyModifiers::CONTROL) => {
            use clipboard::ClipboardProvider;
            match clipboard::ClipboardContext::new().and_then(|mut ctx| ctx.get_contents()) {
                Ok(text) => app.note_input_buffer.push_str(&text),
                Err(e) => app.set_status(format!("Paste error: {e}")),
            }
        }
        KeyCode::Char(c) => {
            app.note_input_buffer.push(c);
        }
        _ => {}
    }
}

fn handle_search_input(app: &mut CodeWalkApp, code: KeyCode) {
    match code {
        KeyCode::Enter => {
            // Simple search: scroll code panel to first match
            let query = app.search_query.to_lowercase();
            if let Some(step) = app.current_step_data() {
                for (i, line) in step.file_content.lines().enumerate() {
                    if line.to_lowercase().contains(&query) {
                        app.code_scroll = i as u16;
                        app.set_status(format!("Found at line {}", i + 1));
                        break;
                    }
                }
            }
            app.mode = CWInputMode::Normal;
        }
        KeyCode::Esc => {
            app.search_query.clear();
            app.mode = CWInputMode::Normal;
        }
        KeyCode::Backspace => {
            app.search_query.pop();
        }
        KeyCode::Char(c) => {
            app.search_query.push(c);
        }
        _ => {}
    }
}

fn handle_deep_dive_list(
    app: &mut CodeWalkApp,
    code: KeyCode,
    stream_tx: &mpsc::UnboundedSender<StreamEvent>,
    api_config: &types::ApiConfig,
    system_prompt: &str,
) {
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            if app.deep_dive_cursor + 1 < app.all_deep_dives.len() {
                app.deep_dive_cursor += 1;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.deep_dive_cursor = app.deep_dive_cursor.saturating_sub(1);
        }
        KeyCode::Enter => {
            if let Some((_, dd)) = app.all_deep_dives.get(app.deep_dive_cursor) {
                let label = dd.label.clone();
                app.mode = CWInputMode::Normal;
                request_deep_dive(app, &label, stream_tx, api_config, system_prompt);
            }
        }
        KeyCode::Esc => {
            app.mode = CWInputMode::Normal;
        }
        _ => {}
    }
}
