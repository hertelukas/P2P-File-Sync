use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Widget, Wrap},
    Frame,
};

use crate::app::{App, CurrentScreen};

pub fn ui(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(frame.area());

    frame.render_widget(folders_block(app), chunks[0]);
    frame.render_widget(peers_block(app), chunks[1]);

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

fn folders_block(app: &App) -> impl Widget {
    let mut list_items = Vec::<ListItem>::new();

    if let Some(config) = app.config.lock().unwrap().clone() {
        let mut i = 0;
        for folder in config.paths() {
            list_items.push(ListItem::new(Line::from(Span::styled(
                format!("{}", folder),
                app.selected_folder
                    .map_or(Style::default(), |selected_folder| {
                        if selected_folder == i {
                            Style::default().bg(Color::DarkGray)
                        } else {
                            Style::default()
                        }
                    }),
            ))));
            i += 1;
        }
    }

    let list = List::new(list_items);
    list.block(
        Block::bordered()
            .borders(Borders::ALL)
            .title_top(Line::from("| Folders |").centered())
            .title_style(Style::default().bold())
            .style(Style::default().fg(match app.current_focus {
                crate::app::CurrentFocus::Folder => Color::Blue,
                _ => Color::default(),
            }))
            .title_bottom(match app.current_mode {
                crate::app::CurrentMode::Insert => " I ",
                crate::app::CurrentMode::Normal => " N ",
            }),
    )
}

fn peers_block(app: &App) -> impl Widget {
    let mut list_items = Vec::<ListItem>::new();

    if let Some(config) = app.config.lock().unwrap().clone() {
        let mut j = 0;
        for peer in config.peers() {
            let lines: Vec<Line> = format!("{}", peer)
                .lines()
                .enumerate()
                .map(|(i, line)| {
                    if i == 0 {
                        Line::from(Span::styled(line.to_string(), Style::default().bold()))
                    } else {
                        Line::from(Span::raw(format!("- {}", line)))
                    }
                })
                .map(|line| {
                    if let Some(selected_peer) = app.selected_peer {
                        if selected_peer == j {
                            line.bg(Color::DarkGray)
                        } else {
                            line
                        }
                    } else {
                        line
                    }
                })
                .collect();

            list_items.push(ListItem::new(lines));
            j += 1;
        }
    }

    let list = List::new(list_items);
    list.block(
        Block::bordered()
            .borders(Borders::ALL)
            .title_top(Line::from("| Peers |").centered())
            .title_style(Style::default().bold())
            .style(Style::default().fg(match app.current_focus {
                crate::app::CurrentFocus::Peer => Color::Blue,
                _ => Color::default(),
            })),
    )
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
