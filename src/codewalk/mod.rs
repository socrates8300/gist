pub mod app;
pub mod claude;
pub mod export;
#[cfg(feature = "meerkat")]
pub mod meerkat_spike;
#[cfg(feature = "meerkat")]
pub mod agent;
#[cfg(feature = "meerkat")]
pub mod recon;
pub mod prompt;
pub mod repo;
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
        match recon::run_recon(&api_config, &repo_path, 4000).await {
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

    // Initialize app state
    let mut app = CodeWalkApp::new(scope.clone(), repo_path.clone(), output_path);

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

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Create streaming channel
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel::<StreamEvent>();

    // Send initial message to Claude
    let init_message = build_init_message(&scope, &repo_summary, repo_map.as_ref());
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
        let summary = export::export_session(&app, &model_str);
        std::fs::write(output_path, &summary)?;
        eprintln!("Session exported to {}", output_path.display());
    }

    // Auto-save session log to ~/.config/gist/sessions/
    if !app.steps.is_empty() {
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
        let session_data = serde_json::json!({
            "repo_path": repo_path.display().to_string(),
            "model": model_str,
            "step_count": app.steps.len(),
            "steps": app.steps.iter().map(|s| serde_json::json!({
                "file": s.response.file,
                "line_start": s.response.line_start,
                "line_end": s.response.line_end,
                "explanation_preview": s.response.explanation.chars().take(200).collect::<String>(),
            })).collect::<Vec<_>>(),
            "timestamp": chrono::Local::now().to_rfc3339(),
        });
        let filename = format!("{ts}-{slug}-walk.json");
        if let Ok(json) = serde_json::to_string_pretty(&session_data) {
            let _ = std::fs::write(sessions_dir.join(filename), json);
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
            handle_note_input(app, code);
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
        KeyCode::Char('j') => {
            app.explanation_scroll = app.explanation_scroll.saturating_add(1);
        }
        KeyCode::Char('k') => {
            app.explanation_scroll = app.explanation_scroll.saturating_sub(1);
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
            app.note_input_buffer.clear();
            app.mode = CWInputMode::NoteInput;
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

fn handle_note_input(app: &mut CodeWalkApp, code: KeyCode) {
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
