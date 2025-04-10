use crate::{Config, Gist, Theme, delete_gist, get_gist, insert_gist, update_gist};
use chrono::Local;
use clipboard::{ClipboardContext, ClipboardProvider};
use colored::Colorize;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
    Frame, Terminal,
};
use rusqlite::Connection;
use std::{
    error::Error,
    io,
    process::Command,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

// ----- UI Result type -----
#[derive(Debug)]
pub enum UIResult {
    Modified,
    NoChanges,
    Error(String),
}

// ----- Input modes -----
#[derive(Debug, PartialEq, Clone)]
enum InputMode {
    Normal,
    Searching,
    Confirming(ConfirmAction),
    TagEditing,
    Help,
}

// ----- Confirmation actions -----
#[derive(Debug, PartialEq, Clone)]
enum ConfirmAction {
    Delete(i64),
    Quit,
}

// ----- App state -----
struct AppState {
    all_gists: Vec<Gist>,
    filtered_gists: Vec<Gist>,
    selected: usize,
    list_state: ListState,
    mode: InputMode,
    search_query: String,
    edit_buffer: String,
    status_message: Option<(String, Instant)>,
    modified: bool,
    help_scroll: u16,
    config: Config,
    focused_panel: Panel,
}

#[derive(Debug, PartialEq)]
enum Panel {
    List,
    Content,
}

impl AppState {
    fn new(gists: Vec<Gist>, config: Config) -> Self {
        let mut s = AppState {
            filtered_gists: gists.clone(),
            all_gists: gists,
            selected: 0,
            list_state: ListState::default(),
            mode: InputMode::Normal,
            search_query: String::new(),
            edit_buffer: String::new(),
            status_message: None,
            modified: false,
            help_scroll: 0,
            config,
            focused_panel: Panel::List,
        };
        if !s.filtered_gists.is_empty() {
            s.list_state.select(Some(0));
        }
        s
    }
    
    fn reload(&mut self, gists: Vec<Gist>) {
        self.all_gists = gists;
        self.filtered_gists = self.all_gists.clone();
        self.selected = 0;
        if !self.filtered_gists.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }
    
    fn reset_filter(&mut self) {
        self.filtered_gists = self.all_gists.clone();
        self.selected = 0;
        if !self.filtered_gists.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
        self.search_query.clear();
    }
    
    fn do_search(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered_gists = self
            .all_gists
            .iter()
            .filter(|g| {
                g.content.to_lowercase().contains(&q) || 
                g.tags.to_lowercase().contains(&q) ||
                g.id.to_string().contains(&q)
            })
            .cloned()
            .collect();
            
        self.selected = 0;
        if !self.filtered_gists.is_empty() {
            self.list_state.select(Some(0));
        } else {
            self.list_state.select(None);
        }
    }
    
    fn set_status(&mut self, msg: String) {
        self.status_message = Some((msg, Instant::now()));
    }
    
    fn get_status(&self) -> Option<String> {
        if let Some((msg, time)) = &self.status_message {
            if time.elapsed() < Duration::from_secs(5) {
                return Some(msg.clone());
            }
        }
        None
    }
    
    fn select_next(&mut self) {
        if self.filtered_gists.is_empty() {
            return;
        }
        
        let i = match self.list_state.selected() {
            Some(i) => {
                if i >= self.filtered_gists.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.selected = i;
        self.list_state.select(Some(i));
    }
    
    fn select_prev(&mut self) {
        if self.filtered_gists.is_empty() {
            return;
        }
        
        let i = match self.list_state.selected() {
            Some(i) => {
                if i == 0 {
                    self.filtered_gists.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.selected = i;
        self.list_state.select(Some(i));
    }
    
    fn current_gist(&self) -> Option<&Gist> {
        self.filtered_gists.get(self.selected)
    }
    
    fn selected_id(&self) -> Option<i64> {
        self.current_gist().map(|g| g.id)
    }
    
    fn toggle_panel(&mut self) {
        self.focused_panel = match self.focused_panel {
            Panel::List => Panel::Content,
            Panel::Content => Panel::List,
        };
    }
}

// UI rendering functions
// UI rendering functions
fn render_ui(f: &mut Frame, state: &mut AppState) {
    match &state.mode {
        InputMode::Help => render_help(f, state),
        InputMode::Confirming(action) => {
            // Clone the action to avoid borrowing issues
            let action_clone = action.clone();
            
            // First render the main UI
            render_main(f, state);
            
            // Create a centered popup
            let area = centered_rect(60, 20, f.area());
            
            // Render the background of the popup
            f.render_widget(Clear, area);
            
            // Render border and title
            let popup_block = Block::default()
                .title("Confirm")
                .borders(Borders::ALL)
                .style(Style::default().bg(Color::Black));
                
            let inner = popup_block.inner(area);
            f.render_widget(popup_block, area);
            
            // Determine the text based on the cloned action
            let text = match action_clone {
                ConfirmAction::Delete(id) => {
                    format!("Are you sure you want to delete gist #{}?\n\nPress y to confirm or Esc to cancel.", id)
                }
                ConfirmAction::Quit => {
                    if state.modified {
                        "You have unsaved changes. Quit anyway?\n\nPress y to confirm or Esc to cancel.".to_string()
                    } else {
                        "Are you sure you want to quit?\n\nPress y to confirm or Esc to cancel.".to_string()
                    }
                }
            };
            
            // Render the confirmation text
            let paragraph = Paragraph::new(text)
                .style(Style::default())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });
                
            f.render_widget(paragraph, inner);
        },
        _ => render_main(f, state),
    }
}

fn render_main(f: &mut Frame, state: &mut AppState) {
    let size = f.area();
    
    // Create layout
    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(size);
    
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(vert[0]);
    
    // Render list panel
    let list_block = Block::default()
        .borders(Borders::ALL)
        .title("Gists")
        .border_style(match state.focused_panel {
            Panel::List => Style::default().fg(Color::Yellow),
            _ => Style::default(),
        });
    
    let items: Vec<_> = state
        .filtered_gists
        .iter()
        .map(|g| {
            let display = format!("#{} {}", g.id, g.tags);
            ListItem::new(display)
        })
        .collect();
    
    let list = List::new(items)
        .block(list_block)
        .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD));
    
    f.render_stateful_widget(list, chunks[0], &mut state.list_state);
    
    // Render content panel
    let content_text = if let Some(gist) = state.current_gist() {
        format!("{}\n\n{}", gist.content, format!("Created: {}", gist.created_at))
    } else {
        "(no gists)".to_string()
    };
    
    let content_block = Block::default()
        .borders(Borders::ALL)
        .title(if let Some(gist) = state.current_gist() {
            format!("Content (ID: {})", gist.id)
        } else {
            "Content".to_string()
        })
        .border_style(match state.focused_panel {
            Panel::Content => Style::default().fg(Color::Yellow),
            _ => Style::default(),
        });
    
    let paragraph = Paragraph::new(content_text)
        .block(content_block)
        .wrap(Wrap { trim: false });
    
    f.render_widget(paragraph, chunks[1]);
    
    // Render status bar
    let status = if let Some(msg) = state.get_status() {
        msg
    } else if state.mode == InputMode::Searching {
        format!("/ {}", state.search_query)
    } else if state.mode == InputMode::TagEditing {
        format!("Edit Tags: {}", state.edit_buffer)
    } else {
        "↑↓ j/k:Navigate  Tab:Switch Panel  a:Add  e:Edit  d:Delete  t:Edit Tags  y:Copy  s/:Search  ?:Help  q:Quit".to_string()
    };
    
    let status_style = if state.mode == InputMode::Normal {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::Green)
    };
    
    let bar = Paragraph::new(status).style(status_style);
    f.render_widget(bar, vert[1]);
}

fn render_help(f: &mut Frame, state: &mut AppState) {
    let size = f.area();
    
    let block = Block::default()
        .title("Help & Keyboard Shortcuts")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));
    
    let inner = block.inner(size);
    f.render_widget(block, size);
    
    let help_text = vec![
        "Navigation:",
        "  ↑/↓, j/k     - Move selection up/down",
        "  PgUp/PgDown  - Move by page",
        "  Home/End     - Jump to start/end",
        "  Tab          - Switch between list and content panels",
        "",
        "Actions:",
        "  a            - Add new snippet",
        "  e            - Edit selected snippet",
        "  d            - Delete selected snippet (with confirmation)",
        "  y            - Copy snippet content to clipboard",
        "  t            - Edit tags for the selected snippet",
        "  r            - Refresh snippet list",
        "",
        "Search:",
        "  s, /         - Start search mode",
        "  Esc          - Exit search/help mode or cancel action",
        "  Enter        - Execute search",
        "",
        "UI:",
        "  ?            - Toggle this help screen",
        "  q            - Quit (with confirmation if changes)",
        "",
        "Press ESC to return",
    ];
    
    let text = Text::from(help_text.join("\n"));
    let paragraph = Paragraph::new(text)
        .style(Style::default())
        .scroll((state.help_scroll, 0))
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Left);
    
    f.render_widget(paragraph, inner);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

// Main UI function
pub fn run_ui(
    gists_storage: &mut Vec<Gist>, 
    conn: Connection,
    config: Config
) -> Result<UIResult, Box<dyn Error>> {
    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    
    // Create channels for background operations
    let (tx, rx) = mpsc::channel();
    let (db_tx, db_rx) = mpsc::channel();
    
    // Create thread-safe connection
    let conn_thread = Arc::new(Mutex::new(conn));
    let conn_ui = Arc::clone(&conn_thread);
    
    // Spawn a thread to handle database operations
    thread::spawn(move || {
        while let Ok(db_op) = db_rx.recv() {
            let conn_lock = conn_thread.lock().unwrap();
            match db_op {
                DbOperation::Add(content, tags, sender) => {
                    let result = insert_gist(&conn_lock, &content, &tags);
                    let _ = sender.send(result.map_err(|e| e.to_string()));
                }
                DbOperation::Update(id, content, tags, sender) => {
                    let result = update_gist(&conn_lock, id, &content, &tags);
                    let _ = sender.send(result.map_err(|e| e.to_string()));
                }
                DbOperation::Delete(id, sender) => {
                    let result = delete_gist(&conn_lock, id);
                    let _ = sender.send(result.map_err(|e| e.to_string()));
                }
                DbOperation::Get(id, sender) => {
                    let result = get_gist(&conn_lock, id);
                    let _ = sender.send(result.map_err(|e| e.to_string()));
                }
            }
        }
    });

    // Setup initial state
    let initial_data = gists_storage.clone();
    let mut state = AppState::new(initial_data, config);
    
    // Set initial status
    state.set_status(format!("Loaded {} gists", state.all_gists.len()));

    // Main loop
    let mut result = UIResult::NoChanges;
    
    loop {
        // Draw UI
        terminal.draw(|f| render_ui(f, &mut state))?;
        
        // Check for background operation results
        if let Ok(op_result) = rx.try_recv() {
            match op_result {
                OperationResult::Add(id) => {
                    let conn_lock = conn_ui.lock().unwrap();
                    if let Ok(Some(gist)) = get_gist(&conn_lock, id) {
                        state.all_gists.push(gist.clone());
                        gists_storage.push(gist);
                        state.reset_filter();
                        state.modified = true;
                        state.set_status(format!("Added gist #{}", id));
                    }
                }
                OperationResult::Update(id) => {
                    let conn_lock = conn_ui.lock().unwrap();
                    if let Ok(Some(gist)) = get_gist(&conn_lock, id) {
                        // Update in both lists
                        for g in state.all_gists.iter_mut() {
                            if g.id == id {
                                *g = gist.clone();
                                break;
                            }
                        }
                        for g in state.filtered_gists.iter_mut() {
                            if g.id == id {
                                *g = gist.clone();
                                break;
                            }
                        }
                        for g in gists_storage.iter_mut() {
                            if g.id == id {
                                *g = gist.clone();
                                break;
                            }
                        }
                        state.modified = true;
                        state.set_status(format!("Updated gist #{}", id));
                    }
                }
                OperationResult::Delete(id, success) => {
                    if success {
                        // Remove from both lists
                        state.all_gists.retain(|g| g.id != id);
                        state.filtered_gists.retain(|g| g.id != id);
                        gists_storage.retain(|g| g.id != id);
                        
                        // Update selection
                        if state.selected >= state.filtered_gists.len() && state.selected > 0 {
                            state.selected = state.filtered_gists.len().saturating_sub(1);
                            state.list_state.select(Some(state.selected));
                        }
                        
                        state.modified = true;
                        state.set_status(format!("Deleted gist #{}", id));
                    } else {
                        state.set_status(format!("Failed to delete gist #{}", id));
                    }
                }
                OperationResult::Error(msg) => {
                    state.set_status(format!("Error: {}", msg));
                }
            }
        }
        
        // Handle input
        if crossterm::event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                match state.mode.clone() {
                    InputMode::Normal => {
                        match key.code {
                            KeyCode::Char('q') => {
                                if state.modified {
                                    state.mode = InputMode::Confirming(ConfirmAction::Quit);
                                } else {
                                    break;
                                }
                            },
                            KeyCode::Char('?') => {
                                state.mode = InputMode::Help;
                                state.help_scroll = 0;
                            },
                            KeyCode::Char('s') | KeyCode::Char('/') => {
                                state.mode = InputMode::Searching;
                                state.search_query.clear();
                            },
                            KeyCode::Char('a') => {
                                // Add new gist
                                disable_raw_mode()?;
                                let tmp = std::env::temp_dir().join("gist_new.txt");
                                let editor = if state.config.editor.is_empty() {
                                    "nvim".to_string()
                                } else {
                                    state.config.editor.clone()
                                };
                                let _ = Command::new(&editor).arg(&tmp).status();
                                if let Ok(content) = std::fs::read_to_string(&tmp) {
                                    let _ = std::fs::remove_file(&tmp);
                                    if !content.trim().is_empty() {
                                        // Add in background
                                        let db_sender = db_tx.clone();
                                        let sender = tx.clone();
                                        thread::spawn(move || {
                                            let (response_tx, response_rx) = mpsc::channel();
                                            let _ = db_sender.send(DbOperation::Add(
                                                content, 
                                                "".to_string(), 
                                                response_tx
                                            ));
                                            
                                            match response_rx.recv() {
                                                Ok(Ok(id)) => {
                                                    let _ = sender.send(OperationResult::Add(id));
                                                }
                                                Ok(Err(e)) => {
                                                    let _ = sender.send(OperationResult::Error(e));
                                                }
                                                Err(_) => {
                                                    let _ = sender.send(OperationResult::Error(
                                                        "Failed to communicate with database thread".to_string()
                                                    ));
                                                }
                                            }
                                        });
                                    }
                                }
                                enable_raw_mode()?;
                            },
                            KeyCode::Char('e') => {
                                if let Some(gist) = state.current_gist().cloned() {
                                    disable_raw_mode()?;
                                    let tmp = std::env::temp_dir().join("gist_edit.txt");
                                    let _ = std::fs::write(&tmp, &gist.content);
                                    let editor = if state.config.editor.is_empty() {
                                        "nvim".to_string()
                                    } else {
                                        state.config.editor.clone()
                                    };
                                    let _ = Command::new(&editor).arg(&tmp).status();
                                    if let Ok(updated) = std::fs::read_to_string(&tmp) {
                                        let _ = std::fs::remove_file(&tmp);
                                        if !updated.trim().is_empty() && updated != gist.content {
                                            // Update in background
                                            let db_sender = db_tx.clone();
                                            let sender = tx.clone();
                                            let id = gist.id;
                                            let tags = gist.tags.clone();
                                            thread::spawn(move || {
                                                let (response_tx, response_rx) = mpsc::channel();
                                                let _ = db_sender.send(DbOperation::Update(
                                                    id, 
                                                    updated, 
                                                    tags, 
                                                    response_tx
                                                ));
                                                
                                                match response_rx.recv() {
                                                    Ok(Ok(_)) => {
                                                        let _ = sender.send(OperationResult::Update(id));
                                                    }
                                                    Ok(Err(e)) => {
                                                        let _ = sender.send(OperationResult::Error(e));
                                                    }
                                                    Err(_) => {
                                                        let _ = sender.send(OperationResult::Error(
                                                            "Failed to communicate with database thread".to_string()
                                                        ));
                                                    }
                                                }
                                            });
                                        }
                                    }
                                    enable_raw_mode()?;
                                } else {
                                    state.set_status("No gist selected".to_string());
                                }
                            },
                            KeyCode::Char('d') => {
                                if let Some(id) = state.selected_id() {
                                    state.mode = InputMode::Confirming(ConfirmAction::Delete(id));
                                } else {
                                    state.set_status("No gist selected".to_string());
                                }
                            },
                            KeyCode::Char('t') => {
                                // Get tags before changing mode to avoid borrow issues
                                let tags = state.current_gist().map(|g| g.tags.clone());
                                
                                if let Some(current_tags) = tags {
                                    state.edit_buffer = current_tags;
                                    state.mode = InputMode::TagEditing;
                                } else {
                                    state.set_status("No gist selected".to_string());
                                }
                            },
                            KeyCode::Char('y') => {
                                if let Some(gist) = state.current_gist() {
                                    if let Ok(mut ctx) = ClipboardContext::new() {
                                        if let Ok(_) = ctx.set_contents(gist.content.clone()) {
                                            state.set_status("Copied to clipboard".to_string());
                                        } else {
                                            state.set_status("Failed to copy to clipboard".to_string());
                                        }
                                    } else {
                                        state.set_status("Clipboard not available".to_string());
                                    }
                                } else {
                                    state.set_status("No gist selected".to_string());
                                }
                            },
                            KeyCode::Char('r') => {
                                // Reload from database
                                let conn_lock = conn_ui.lock().unwrap();
                                let result = crate::list_gists(&conn_lock, usize::MAX, "created_at");
                                match result {
                                    Ok(gists) => {
                                        *gists_storage = gists.clone();
                                        state.reload(gists);
                                        state.set_status(format!("Reloaded {} gists", gists_storage.len()));
                                    }
                                    Err(e) => {
                                        state.set_status(format!("Error: {}", e));
                                    }
                                }
                            },
                            KeyCode::Down | KeyCode::Char('j') => {
                                state.select_next();
                            },
                            KeyCode::Up | KeyCode::Char('k') => {
                                state.select_prev();
                            },
                            KeyCode::Tab => {
                                state.toggle_panel();
                            },
                            KeyCode::PageDown => {
                                // Jump 10 items
                                for _ in 0..10 {
                                    state.select_next();
                                }
                            },
                            KeyCode::PageUp => {
                                // Jump 10 items
                                for _ in 0..10 {
                                    state.select_prev();
                                }
                            },
                            KeyCode::Home => {
                                // Jump to first item
                                if !state.filtered_gists.is_empty() {
                                    state.selected = 0;
                                    state.list_state.select(Some(0));
                                }
                            },
                            KeyCode::End => {
                                // Jump to last item
                                if !state.filtered_gists.is_empty() {
                                    state.selected = state.filtered_gists.len() - 1;
                                    state.list_state.select(Some(state.selected));
                                }
                            },
                            _ => {}
                        }
                    },
                    InputMode::Searching => {
                        match key.code {
                            KeyCode::Esc => {
                                state.mode = InputMode::Normal;
                                state.reset_filter();
                            },
                            KeyCode::Enter => {
                                state.mode = InputMode::Normal;
                                state.do_search();
                            },
                            KeyCode::Backspace => {
                                state.search_query.pop();
                            },
                            KeyCode::Char(c) => {
                                state.search_query.push(c);
                            },
                            _ => {}
                        }
                    },
                    InputMode::TagEditing => {
                        match key.code {
                            KeyCode::Esc => {
                                state.mode = InputMode::Normal;
                                state.edit_buffer.clear();
                            },
                            KeyCode::Enter => {
                                if let Some(gist) = state.current_gist().cloned() {
                                    let id = gist.id;
                                    let content = gist.content.clone();
                                    let tags = state.edit_buffer.clone();
                                    
                                    // Update tags in background
                                    let db_sender = db_tx.clone();
                                    let sender = tx.clone();
                                    thread::spawn(move || {
                                        let (response_tx, response_rx) = mpsc::channel();
                                        let _ = db_sender.send(DbOperation::Update(
                                            id, 
                                            content, 
                                            tags, 
                                            response_tx
                                        ));
                                        
                                        match response_rx.recv() {
                                            Ok(Ok(_)) => {
                                                let _ = sender.send(OperationResult::Update(id));
                                            }
                                            Ok(Err(e)) => {
                                                let _ = sender.send(OperationResult::Error(e));
                                            }
                                            Err(_) => {
                                                let _ = sender.send(OperationResult::Error(
                                                    "Failed to communicate with database thread".to_string()
                                                ));
                                            }
                                        }
                                    });
                                }
                                state.mode = InputMode::Normal;
                                state.edit_buffer.clear();
                            },
                            KeyCode::Backspace => {
                                state.edit_buffer.pop();
                            },
                            KeyCode::Char(c) => {
                                state.edit_buffer.push(c);
                            },
                            _ => {}
                        }
                    },
                    InputMode::Help => {
                        match key.code {
                            KeyCode::Esc | KeyCode::Char('?') => {
                                state.mode = InputMode::Normal;
                            },
                            KeyCode::Up => {
                                if state.help_scroll > 0 {
                                    state.help_scroll -= 1;
                                }
                            },
                            KeyCode::Down => {
                                state.help_scroll += 1;
                            },
                            KeyCode::PageUp => {
                                if state.help_scroll > 10 {
                                    state.help_scroll -= 10;
                                } else {
                                    state.help_scroll = 0;
                                }
                            },
                            KeyCode::PageDown => {
                                state.help_scroll += 10;
                            },
                            _ => {}
                        }
                    },
                                        InputMode::Confirming(action) => {
                        match key.code {
                            KeyCode::Esc => {
                                state.mode = InputMode::Normal;
                            },
                            KeyCode::Char('y') => {
                                match action {
                                    ConfirmAction::Quit => {
                                        // Exit the UI
                                        if state.modified {
                                            result = UIResult::Modified;
                                        }
                                        break;
                                    },
                                    ConfirmAction::Delete(id) => {
                                        // Delete in background
                                        let db_sender = db_tx.clone();
                                        let sender = tx.clone();
                                        let delete_id = id;
                                        thread::spawn(move || {
                                            let (response_tx, response_rx) = mpsc::channel();
                                            let _ = db_sender.send(DbOperation::Delete(
                                                delete_id, 
                                                response_tx
                                            ));
                                            
                                            match response_rx.recv() {
                                                Ok(Ok(success)) => {
                                                    let _ = sender.send(OperationResult::Delete(delete_id, success));
                                                }
                                                Ok(Err(e)) => {
                                                    let _ = sender.send(OperationResult::Error(e));
                                                }
                                                Err(_) => {
                                                    let _ = sender.send(OperationResult::Error(
                                                        "Failed to communicate with database thread".to_string()
                                                    ));
                                                }
                                            }
                                        });
                                        state.mode = InputMode::Normal;
                                    }
                                }
                            },
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Clean up terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    
    Ok(result)
}

// Database operation message types
enum DbOperation {
    Add(String, String, mpsc::Sender<Result<i64, String>>),
    Update(i64, String, String, mpsc::Sender<Result<(), String>>),
    Delete(i64, mpsc::Sender<Result<bool, String>>),
    Get(i64, mpsc::Sender<Result<Option<Gist>, String>>),
}

// Operation result types
enum OperationResult {
    Add(i64),
    Update(i64),
    Delete(i64, bool),
    Error(String),
}

