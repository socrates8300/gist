use crate::codewalk::app::{CWInputMode, CWPanel, CodeWalkApp};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Style as SyntectStyle, ThemeSet};
use syntect::parsing::SyntaxSet;

/// Main render dispatcher
pub fn render_codewalk(f: &mut Frame, app: &mut CodeWalkApp) {
    match &app.mode {
        CWInputMode::Help => render_help_overlay(f, app),
        CWInputMode::DeepDiveList => {
            render_main(f, app);
            render_deep_dive_popup(f, app);
        }
        CWInputMode::ConfirmQuit => {
            render_main(f, app);
            render_confirm_quit(f);
        }
        _ => render_main(f, app),
    }
}

/// Render the main three-panel layout
fn render_main(f: &mut Frame, app: &mut CodeWalkApp) {
    let size = f.area();

    // Vertical: main area + optional tech debt + status bar
    let mut vert_constraints = vec![Constraint::Min(0)];
    if app.tech_debt_visible && !app.tech_debt_notes.is_empty() {
        let debt_height = (app.tech_debt_notes.len() as u16 + 2).min(8);
        vert_constraints.insert(0, Constraint::Min(0));
        vert_constraints[0] = Constraint::Min(0);
        vert_constraints.push(Constraint::Length(debt_height));
    }
    vert_constraints.push(Constraint::Length(1));

    let vert = Layout::default()
        .direction(Direction::Vertical)
        .constraints(&vert_constraints)
        .split(size);

    // Top section: Code (left) + Explanation (right)
    let main_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vert[0]);

    render_code_panel(f, app, main_chunks[0]);
    render_explanation_panel(f, app, main_chunks[1]);

    // Tech debt panel (if visible)
    if app.tech_debt_visible && !app.tech_debt_notes.is_empty() {
        render_tech_debt_panel(f, app, vert[1]);
        render_status_bar(f, app, vert[2]);
    } else {
        render_status_bar(f, app, vert[1]);
    }

    // Note input overlay at bottom
    if app.mode == CWInputMode::NoteInput {
        let input_area = Rect {
            x: 0,
            y: size.height.saturating_sub(3),
            width: size.width,
            height: 3,
        };
        f.render_widget(Clear, input_area);
        let input_block = Block::default()
            .borders(Borders::ALL)
            .title("Tech Debt Note (Enter to save, Esc to cancel)")
            .border_style(Style::default().fg(Color::Yellow));
        let input_text = Paragraph::new(app.note_input_buffer.as_str()).block(input_block);
        f.render_widget(input_text, input_area);
    }

    // Search input overlay
    if app.mode == CWInputMode::SearchInFile {
        let input_area = Rect {
            x: 0,
            y: size.height.saturating_sub(3),
            width: size.width,
            height: 3,
        };
        f.render_widget(Clear, input_area);
        let input_block = Block::default()
            .borders(Borders::ALL)
            .title("Search (Enter to find, Esc to cancel)")
            .border_style(Style::default().fg(Color::Cyan));
        let input_text = Paragraph::new(app.search_query.as_str()).block(input_block);
        f.render_widget(input_text, input_area);
    }
}

/// Render the code panel with syntax highlighting and line-range highlighting
fn render_code_panel(f: &mut Frame, app: &CodeWalkApp, area: Rect) {
    let file_name = app.current_file().unwrap_or("No file");
    let step_info = if !app.steps.is_empty() {
        format!(
            " [{}/{}]",
            app.current_step + 1,
            app.steps.len()
        )
    } else {
        String::new()
    };

    let title = format!("CODE  {}{}", file_name, step_info);
    let border_style = if app.focused_panel == CWPanel::Code {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style);

    let code = app.current_code();
    if code.is_empty() {
        let empty = Paragraph::new("(no file loaded)")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, area);
        return;
    }

    let highlight_range = app.highlight_range();
    let inner = block.inner(area);

    // Build syntax-highlighted lines
    let ps = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();
    let ext = app
        .current_file()
        .and_then(|f| f.rsplit('.').next())
        .unwrap_or("txt");
    let syntax = ps
        .find_syntax_by_extension(ext)
        .unwrap_or_else(|| ps.find_syntax_plain_text());
    let mut h = HighlightLines::new(syntax, &ts.themes["base16-ocean.dark"]);

    let mut lines: Vec<Line> = Vec::new();
    for (i, line_text) in code.lines().enumerate() {
        let line_num = i + 1;
        let is_highlighted = highlight_range
            .map(|(start, end)| line_num >= start && line_num <= end)
            .unwrap_or(false);

        // Line number
        let num_style = if is_highlighted {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let mut spans = vec![Span::styled(format!("{:>4} ", line_num), num_style)];

        // Syntax-highlighted code text
        if let Ok(ranges) = h.highlight_line(line_text, &ps) {
            for (style, text) in ranges {
                let fg = syntect_to_ratatui_color(style);
                let mut ratatui_style = Style::default().fg(fg);
                if is_highlighted {
                    ratatui_style = ratatui_style.bg(Color::Rgb(40, 40, 60));
                }
                spans.push(Span::styled(text.to_string(), ratatui_style));
            }
        } else {
            let style = if is_highlighted {
                Style::default().bg(Color::Rgb(40, 40, 60))
            } else {
                Style::default()
            };
            spans.push(Span::styled(line_text.to_string(), style));
        }

        lines.push(Line::from(spans));
    }

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text)
        .block(block)
        .scroll((app.code_scroll, 0));

    f.render_widget(paragraph, area);
}

/// Render the explanation panel
fn render_explanation_panel(f: &mut Frame, app: &CodeWalkApp, area: Rect) {
    let border_style = if app.focused_panel == CWPanel::Explanation {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if app.is_streaming {
        "EXPLANATION (streaming...)"
    } else {
        "EXPLANATION"
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style);

    let explanation = app.current_explanation();

    // Highlight [DEEP DIVE AVAILABLE: ...] markers
    let mut lines: Vec<Line> = Vec::new();
    for text_line in explanation.lines() {
        if text_line.contains("[DEEP DIVE AVAILABLE:") {
            let mut spans = Vec::new();
            let mut remaining = text_line;
            while let Some(start) = remaining.find("[DEEP DIVE AVAILABLE:") {
                if start > 0 {
                    spans.push(Span::raw(remaining[..start].to_string()));
                }
                let end = remaining[start..]
                    .find(']')
                    .map(|e| start + e + 1)
                    .unwrap_or(remaining.len());
                spans.push(Span::styled(
                    remaining[start..end].to_string(),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ));
                remaining = &remaining[end..];
            }
            if !remaining.is_empty() {
                spans.push(Span::raw(remaining.to_string()));
            }
            lines.push(Line::from(spans));
        } else if text_line.starts_with("## ") || text_line.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                text_line.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if text_line.starts_with("- ") || text_line.starts_with("* ") {
            lines.push(Line::from(Span::styled(
                text_line.to_string(),
                Style::default().fg(Color::Green),
            )));
        } else {
            lines.push(Line::from(text_line.to_string()));
        }
    }

    // Streaming cursor
    if app.is_streaming {
        lines.push(Line::from(Span::styled(
            "▌",
            Style::default().fg(Color::Yellow),
        )));
    }

    let text = Text::from(lines);
    let paragraph = Paragraph::new(text)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((app.explanation_scroll, 0));

    f.render_widget(paragraph, area);
}

/// Render the tech debt notes panel
fn render_tech_debt_panel(f: &mut Frame, app: &CodeWalkApp, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title("TECH DEBT NOTES")
        .border_style(Style::default().fg(Color::Red));

    let items: Vec<ListItem> = app
        .tech_debt_notes
        .iter()
        .enumerate()
        .map(|(i, note)| {
            let text = format!(
                "{}. {}:{} — {}",
                i + 1,
                note.file,
                note.line_range,
                note.note
            );
            ListItem::new(text).style(Style::default().fg(Color::White))
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

/// Render the status bar
fn render_status_bar(f: &mut Frame, app: &CodeWalkApp, area: Rect) {
    let status = if let Some(msg) = app.get_status() {
        msg.to_string()
    } else {
        match app.mode {
            CWInputMode::WaitingForStep => "Waiting for Claude...".to_string(),
            CWInputMode::NoteInput => format!("Note: {}", app.note_input_buffer),
            CWInputMode::SearchInFile => format!("/{}", app.search_query),
            _ => {
                "[n]ext [p]rev [d]eep dive [t]ag debt [T]oggle debt [D]ive list [s]earch [?]help [q]uit"
                    .to_string()
            }
        }
    };

    let style = match app.mode {
        CWInputMode::Normal => Style::default().fg(Color::Yellow),
        CWInputMode::WaitingForStep => Style::default().fg(Color::Cyan),
        _ => Style::default().fg(Color::Green),
    };

    let bar = Paragraph::new(status).style(style);
    f.render_widget(bar, area);
}

/// Render help overlay
fn render_help_overlay(f: &mut Frame, _app: &mut CodeWalkApp) {
    let size = f.area();
    f.render_widget(Clear, size);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("CodeWalk Help — press Esc or ? to close")
        .border_style(Style::default().fg(Color::Cyan));

    let help_text = vec![
        Line::from(Span::styled(
            "Step Navigation",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  n       Next step (requests from Claude if at end)"),
        Line::from("  p       Previous step"),
        Line::from("  N       Jump forward 5 steps"),
        Line::from("  P       Jump back 5 steps"),
        Line::from("  gg      Jump to walkthrough start"),
        Line::from("  G       Jump to walkthrough end"),
        Line::from(""),
        Line::from(Span::styled(
            "Scrolling",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  j/k     Scroll explanation down/up"),
        Line::from("  J/K     Scroll code panel down/up"),
        Line::from("  Ctrl-d  Half-page down (focused panel)"),
        Line::from("  Ctrl-u  Half-page up (focused panel)"),
        Line::from("  Tab     Switch focus between panels"),
        Line::from(""),
        Line::from(Span::styled(
            "Actions",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("  d       Deep dive on first available topic"),
        Line::from("  D       List all deep dive topics"),
        Line::from("  t       Write a tech debt note"),
        Line::from("  T       Toggle tech debt panel"),
        Line::from("  s       Search within current file"),
        Line::from("  ?       Toggle this help"),
        Line::from("  q       Quit (with export prompt if --output set)"),
    ];

    let paragraph = Paragraph::new(help_text)
        .block(block)
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, size);
}

/// Render deep dive list popup
fn render_deep_dive_popup(f: &mut Frame, app: &mut CodeWalkApp) {
    let area = centered_rect(70, 60, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Deep Dive Topics — Enter to select, Esc to close")
        .border_style(Style::default().fg(Color::Magenta));

    if app.all_deep_dives.is_empty() {
        let paragraph = Paragraph::new("No deep dive topics discovered yet.")
            .block(block)
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = app
        .all_deep_dives
        .iter()
        .enumerate()
        .map(|(i, (step_idx, dd))| {
            let style = if i == app.deep_dive_cursor {
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(format!("Step {} — {}", step_idx + 1, dd.label)).style(style)
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

/// Render quit confirmation popup
fn render_confirm_quit(f: &mut Frame) {
    let area = centered_rect(50, 20, f.area());
    f.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("Quit")
        .border_style(Style::default().fg(Color::Red));

    let text = Paragraph::new("Quit CodeWalk session?\n\nPress y to confirm, Esc to cancel.")
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(text, area);
}

/// Create a centered rectangle
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

/// Convert a syntect color to ratatui Color
fn syntect_to_ratatui_color(style: SyntectStyle) -> Color {
    Color::Rgb(
        style.foreground.r,
        style.foreground.g,
        style.foreground.b,
    )
}
