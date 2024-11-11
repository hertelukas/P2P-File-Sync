use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Position, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Widget, Wrap},
    Frame,
};

use crate::app::{self, App, CurrentMode, CurrentScreen, EditFolderFocus, EditFolderState};

pub fn ui(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(frame.area());

    // Potentially show popups
    match app.current_screen {
        CurrentScreen::Main => {
            frame.render_widget(folders_block(app), chunks[0]);
            frame.render_widget(peers_block(app), chunks[1]);
        }
        CurrentScreen::Loading => {
            let popup_block = create_popup_block(app, "Loading".to_string(), false);

            let area = centered_rect(50, 50, frame.area());
            frame.render_widget(popup_block, area);
        }
        CurrentScreen::Error(ref msg) => {
            let popup_block = create_popup_block(app, "Error".to_string(), false)
                .style(Style::default().fg(Color::Red));

            let error_text = Text::styled(msg, Style::default().fg(Color::default()));
            let error_paragraph = Paragraph::new(error_text)
                .block(popup_block)
                .alignment(ratatui::layout::Alignment::Center)
                .wrap(Wrap { trim: false }); // Do not cut off whn over edge

            let area = centered_rect(50, 50, frame.area());
            frame.render_widget(error_paragraph, area);
        }
        CurrentScreen::EditFolder(ref edit_folder_state) => {
            render_edit_folder(frame, app, edit_folder_state, "Edit Folder".to_string());
        }
        CurrentScreen::EditPeer(ref peer) => {
            let popup_block = create_popup_block(app, "Edit Peer".to_string(), true);

            let folder_text = Text::styled(
                format!("{}", peer.ip),
                Style::default().fg(Color::default()),
            );
            let folder_paragraph = Paragraph::new(folder_text)
                .block(popup_block)
                .alignment(ratatui::layout::Alignment::Center)
                .wrap(Wrap { trim: false }); // Do not cut off whn over edge

            let area = centered_rect(50, 50, frame.area());
            frame.render_widget(folder_paragraph, area);
        }
        CurrentScreen::CreateFolder(ref create_folder_state) => {
            render_edit_folder(frame, app, create_folder_state, "Create Folder".to_string());
        }
        CurrentScreen::CreatePeer => {
            let popup_block = create_popup_block(app, "Create Peer".to_string(), true);

            let area = centered_rect(50, 50, frame.area());
            frame.render_widget(popup_block, area);
        }
    }
}

fn render_edit_folder(frame: &mut Frame, app: &App, folder: &EditFolderState, title: String) {
    let popup_block = create_popup_block(app, title, true);
    let vertical = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Length(3),
    ]);

    let area = centered_rect(50, 50, frame.area());
    let [help_area, folder_area, id_area] = vertical.areas(area.inner(Margin {
        horizontal: 1,
        vertical: 1,
    }));

    let (msg, style) = match app.current_mode {
        CurrentMode::Insert => (
            vec![
                "Press ".into(),
                "Esc".bold(),
                " to stop editing, ".into(),
                "Enter".bold(),
                " to add the folder.".into(),
            ],
            Style::default().add_modifier(Modifier::RAPID_BLINK),
        ),
        CurrentMode::Normal => (
            vec![
                "Press ".into(),
                "I".bold(),
                " to start editing, ".into(),
                "Tab".bold(),
                " to switch the focus, and ".into(),
                "Enter".bold(),
                " to save the changes.".into(),
            ],
            Style::default(),
        ),
    };
    let text = Text::from(Line::from(msg)).patch_style(style);
    let help_message = Paragraph::new(text);

    let folder_input = Paragraph::new(folder.path_input.text.as_str())
        .style(match folder.focus {
            EditFolderFocus::Folder => Style::default().fg(Color::Blue),
            EditFolderFocus::Id => Style::default(),
        })
        .block(Block::bordered().title("Path"));

    let id_input = Paragraph::new(folder.id_input.text.as_str())
        .style(match folder.focus {
            EditFolderFocus::Folder => Style::default(),
            EditFolderFocus::Id => Style::default().fg(
                if let Ok(_) = (&folder.id_input).try_into() as Result<u32, _> {
                    Color::Blue
                } else {
                    Color::Red
                },
            ),
        })
        .block(Block::bordered().title("ID"));

    frame.render_widget(popup_block, area);
    frame.render_widget(help_message, help_area);
    frame.render_widget(folder_input, folder_area);
    frame.render_widget(id_input, id_area);

    // Render cursor
    match app.current_mode {
        // Hide the cursor
        CurrentMode::Normal => {}
        // Show cursor in correct input area
        #[allow(clippy::cast_possible_truncation)]
        CurrentMode::Insert => frame.set_cursor_position(Position::new(
            match folder.focus {
                EditFolderFocus::Folder => folder_area.x + folder.path_input.index as u16 + 1,

                EditFolderFocus::Id => id_area.x + folder.id_input.index as u16 + 1,
            },
            match folder.focus {
                EditFolderFocus::Folder => folder_area.y + 1,
                EditFolderFocus::Id => id_area.y + 1,
            },
        )),
    };
}

fn create_popup_block(app: &App, title: String, show_mode: bool) -> Block {
    let block = Block::default()
        .title_top(Line::from(format!("| {} |", title)).centered())
        .borders(Borders::ALL);

    if show_mode {
        block.title_bottom(match app.current_mode {
            app::CurrentMode::Insert => " I ",
            app::CurrentMode::Normal => " N ",
        })
    } else {
        block
    }
}

fn folders_block(app: &App) -> impl Widget {
    let mut list_items = Vec::<ListItem>::new();

    if let Some(config) = &app.config {
        let mut i = 0;
        for folder in config.paths() {
            list_items.push(ListItem::new(
                Line::from(Span::raw(format!("{}", folder))).bg(app.selected_folder.map_or(
                    Color::default(),
                    |selected_folder| {
                        if selected_folder == i {
                            Color::DarkGray
                        } else {
                            Color::default()
                        }
                    },
                )),
            ));
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
                app::CurrentFocus::Folder => Color::Blue,
                _ => Color::default(),
            }))
            .title_bottom(match app.current_mode {
                app::CurrentMode::Insert => " I ",
                app::CurrentMode::Normal => " N ",
            }),
    )
}

fn peers_block(app: &App) -> impl Widget {
    let mut list_items = Vec::<ListItem>::new();

    if let Some(config) = &app.config {
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
                app::CurrentFocus::Peer => Color::Blue,
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
