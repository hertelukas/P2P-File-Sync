use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, CurrentScreen};

pub fn ui(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(frame.area());

    let folders_block = Block::bordered()
        .borders(Borders::ALL)
        .title_top(Line::from("| Folders |").centered())
        .title_style(Style::default().bold())
        .style(Style::default().fg(match app.current_focus {
            crate::app::CurrentFocus::File => Color::Blue,
            _ => Color::default(),
        }))
        .title_bottom(match app.current_mode {
            crate::app::CurrentMode::Insert => " I ",
            crate::app::CurrentMode::Normal => " N ",
        });

    let peers_block = Block::bordered()
        .borders(Borders::ALL)
        .title_top(Line::from("| Peers |").centered())
        .title_style(Style::default().bold())
        .style(Style::default().fg(match app.current_focus {
            crate::app::CurrentFocus::Peer => Color::Blue,
            _ => Color::default(),
        }));

    frame.render_widget(folders_block, chunks[0]);
    frame.render_widget(peers_block, chunks[1]);

    // Potentially show loading "popup"
    if let CurrentScreen::Loading = *app.current_screen.lock().unwrap() {
        // Clear the drawn window
        frame.render_widget(Clear, frame.area());

        let popup_block = Block::default()
            .title_top(Line::from("| Loading... |").centered())
            .borders(Borders::ALL)
            .style(Style::default());

        let area = centered_rect(50, 50, frame.area());
        frame.render_widget(popup_block, area);
    }

    // Potentially show loading popup
    if let CurrentScreen::Error(ref msg) = *app.current_screen.lock().unwrap() {
        // Clear the drawn window
        frame.render_widget(Clear, frame.area());

        let popup_block = Block::default()
            .title_top(Line::from("| Error |").centered())
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Red));

        let error_text = Text::styled(msg, Style::default().fg(Color::default()));
        let error_paragraph = Paragraph::new(error_text)
            .block(popup_block)
            .alignment(ratatui::layout::Alignment::Center)
            .wrap(Wrap { trim: false }); // Do not cut off whn over edge

        let area = centered_rect(50, 50, frame.area());
        frame.render_widget(error_paragraph, area);
    }
}

/// helper function to create a centered rect using up certain percentage of the available rect `r`
// Adapted from https://ratatui.rs/tutorials/json-editor/ui/
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    // Cut the given rectangle into three vertical pieces
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    // Then cut the middle vertical piece into three width-wise pieces
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1] // Return the middle chunk
}
