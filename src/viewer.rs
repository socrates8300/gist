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
use std::{error::Error, io};

use crate::Gist;

struct AppState<'a> {
    all_gists: &'a [Gist],
    filtered_gists: Vec<Gist>,
    selected: usize,
    list_state: ListState,
    is_searching: bool,
    search_query: String,
}

impl<'a> AppState<'a> {
    fn new(gists: &'a [Gist]) -> Self {
        let mut s = AppState {
            all_gists: gists,
            filtered_gists: gists.to_vec(),
            selected: 0,
            list_state: ListState::default(),
            is_searching: false,
            search_query: String::new(),
        };
        s.list_state.select(Some(0));
        s
    }

    fn reset_filter(&mut self) {
        self.filtered_gists = self.all_gists.to_vec();
        self.selected = 0;
        self.list_state.select(Some(0));
        self.search_query.clear();
    }

    fn do_search(&mut self) {
        let text = self.search_query.to_lowercase();
        self.filtered_gists = self
            .all_gists
            .iter()
            .filter(|g| {
                g.content.to_lowercase().contains(&text) || g.tags.to_lowercase().contains(&text)
            })
            .cloned()
            .collect();
        self.selected = 0;
        self.list_state.select(Some(0));
    }
}

pub fn run_ui(gists: &[Gist]) -> Result<(), Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = AppState::new(gists);

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

            // List panel
            let list_items: Vec<_> = state
                .filtered_gists
                .iter()
                .map(|g| ListItem::new(format!("#{} {}", g.id, g.tags)))
                .collect();

            let list = List::new(list_items)
                .block(Block::default().borders(Borders::ALL).title("Gists"))
                .highlight_style(Style::default().bg(Color::Blue));

            f.render_stateful_widget(list, chunks[0], &mut state.list_state);

            // Content panel (avoid panic if no gist)
            let content = if let Some(g) = state.filtered_gists.get(state.selected) {
                g.content.as_str()
            } else {
                "(no gists)"
            };

            let para = Paragraph::new(content)
                .block(Block::default().borders(Borders::ALL).title("Content"));

            f.render_widget(para, chunks[1]);

            // Bottom bar: Help or Search
            let bar_text = if state.is_searching {
                format!("/ {}", state.search_query)
            } else {
                "↑↓ or j/k Navigate  a:Add  e:Edit  s or /:Search  y:Yank/Copy q:Quit".to_owned()
            };
            let bar = Paragraph::new(bar_text).style(Style::default().fg(Color::Yellow));
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
                                match clipboard::ClipboardContext::new() {
                                    Ok(mut ctx) => {
                                        if ctx.set_contents(gist.content.clone()).is_ok() {
                                            println!("Gist copied to clipboard");
                                        } else {
                                            println!("Could not copy gist content to clipboard");
                                        }
                                    }
                                    Err(_) => println!("Clipboard unavailable"),
                                }
                            }
                        }
                        KeyCode::Char('a') => {
                            println!("(Add not yet implemented)");
                        }
                        KeyCode::Char('e') => {
                            println!("(Edit not yet implemented)");
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
