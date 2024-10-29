use ratatui::{
    style::{Style, Stylize},
    text::Line,
    widgets::{Block, BorderType, Borders},
    Frame,
};

use crate::app::App;

pub fn ui(frame: &mut Frame, app: &App) {
    let title_block = Block::bordered()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .style(Style::default())
        .title_top(Line::from(" P2P File Sync CLI ").centered())
        .title_style(Style::default().bold());

    frame.render_widget(title_block, frame.area());
}
