use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style, Stylize},
    text::Line,
    widgets::{Block, Borders},
    Frame,
};

use crate::app::App;

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
}
