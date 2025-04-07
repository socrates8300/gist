use crate::Gist;
use chrono::Local;
use clipboard::{ClipboardContext, ClipboardProvider};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use std::{error::Error, io, process::Command};

// ----- App state -----
struct AppState {
    all_gists: Vec<Gist>,
    filtered_gists: Vec<Gist>,
    selected: usize,
    list_state: ListState,
    is_searching: bool,
    search_query: String,
}

impl AppState {
    fn new(gists: Vec<Gist>) -> Self {
        let mut s = AppState {
            filtered_gists: gists.clone(),
            all_gists: gists,
            selected: 0,
            list_state: ListState::default(),
            is_searching: false,
            search_query: String::new(),
        };
        s.list_state.select(Some(0));
        s
    }
    fn reload(&mut self) {
        self.filtered_gists = self.all_gists.clone();
        self.selected = 0;
        self.list_state.select(Some(0));
    }
    fn reset_filter(&mut self) {
        self.reload();
        self.search_query.clear();
    }
    fn do_search(&mut self) {
        let q = self.search_query.to_lowercase();
        self.filtered_gists = self
            .all_gists
            .iter()
            .filter(|g| g.content.to_lowercase().contains(&q) || g.tags.to_lowercase().contains(&q))
            .cloned()
            .collect();
        self.selected = 0;
        self.list_state.select(Some(0));
    }
}

pub fn run_ui(gists_storage: &mut Vec<Gist>) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let initial_data = gists_storage.clone();
    let mut state = AppState::new(initial_data);

    loop {
        terminal.draw(|f| {
            let vert = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(f.area());

            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
                .split(vert[0]);

            let items: Vec<_> = state
                .filtered_gists
                .iter()
                .map(|g| ListItem::new(format!("#{} {}", g.id, g.tags)))
                .collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Gists"))
                .highlight_style(Style::default().bg(Color::Blue));

            f.render_stateful_widget(list, chunks[0], &mut state.list_state);

            let content = state
                .filtered_gists
                .get(state.selected)
                .map(|g| g.content.as_str())
                .unwrap_or("(no gists)");

            let paragraph = Paragraph::new(content)
                .block(Block::default().borders(Borders::ALL).title("Content"));

            f.render_widget(paragraph, chunks[1]);

            let info = if state.is_searching {
                format!("/ {}", state.search_query)
            } else {
                "↑↓ j/k Navigate  a:Add  e:Edit  y:Yank  s or /:Search  q:Quit".to_string()
            };
            let bar = Paragraph::new(info).style(Style::default().fg(Color::Yellow));
            f.render_widget(bar, vert[1]);
        })?;

        if crossterm::event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if state.is_searching {
                    match key.code {
                        KeyCode::Esc => {
                            state.is_searching = false;
                            state.reset_filter();
                        }
                        KeyCode::Enter => {
                            state.is_searching = false;
                            state.do_search();
                        }
                        KeyCode::Backspace => {
                            state.search_query.pop();
                        }
                        KeyCode::Char(c) => {
                            state.search_query.push(c);
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('s') | KeyCode::Char('/') => {
                            state.is_searching = true;
                            state.search_query.clear();
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if state.selected + 1 < state.filtered_gists.len() {
                                state.selected += 1;
                            }
                            state.list_state.select(Some(state.selected));
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if state.selected > 0 {
                                state.selected -= 1;
                            }
                            state.list_state.select(Some(state.selected));
                        }
                        KeyCode::Char('y') => {
                            if let Some(gist) = state.filtered_gists.get(state.selected) {
                                if let Ok(mut ctx) = ClipboardContext::new() {
                                    let _ = ctx.set_contents(gist.content.clone());
                                }
                            }
                        }
                        KeyCode::Char('a') => {
                            disable_raw_mode()?;
                            let tmp = std::env::temp_dir().join("gist_new.txt");
                            let _ = Command::new("nvim").arg(&tmp).status();
                            let content = std::fs::read_to_string(&tmp).unwrap_or_default();
                            std::fs::remove_file(&tmp).ok();
                            if content.trim().is_empty() {
                                enable_raw_mode()?;
                                continue;
                            }
                            let created_at = Local::now().to_rfc3339();
                            let new_id =
                                state.all_gists.iter().map(|g| g.id).max().unwrap_or(0) + 1;
                            let new_gist = Gist {
                                id: new_id,
                                tags: "".to_string(),
                                content,
                                created_at,
                            };
                            state.all_gists.push(new_gist.clone());
                            gists_storage.push(new_gist);
                            enable_raw_mode()?;
                            state.reload();
                        }
                        KeyCode::Char('e') => {
                            if let Some(gist) = state.filtered_gists.get(state.selected) {
                                disable_raw_mode()?;
                                let tmp = std::env::temp_dir().join("gist_edit.txt");
                                let _ = std::fs::write(&tmp, &gist.content);
                                let _ = Command::new("nvim").arg(&tmp).status();
                                let updated = std::fs::read_to_string(&tmp).unwrap_or_default();
                                std::fs::remove_file(&tmp).ok();
                                if updated.trim().is_empty() {
                                    enable_raw_mode()?;
                                    continue;
                                }
                                // update in full list
                                for g in gists_storage.iter_mut() {
                                    if g.id == gist.id {
                                        g.content = updated.clone();
                                    }
                                }
                                enable_raw_mode()?;
                                state.all_gists = gists_storage.clone();
                                state.do_search();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}
