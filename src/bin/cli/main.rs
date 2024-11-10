use app::{App, CreateFolderFocus, CurrentFocus, CurrentMode, CurrentScreen};
use env_logger::Env;
use ratatui::{
    crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    },
    prelude::{Backend, CrosstermBackend},
    Terminal,
};
use std::io;
use ui::ui;

extern crate p2p_file_sync;

mod app;
mod ui;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    // Use info as default logging level
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    color_eyre::install()?;

    init_panic_hook();

    // Setup terminal
    let mut terminal = init_tui()?;
    terminal.clear()?;

    let mut app = App::new();
    let _ = run(&mut terminal, &mut app).await;

    //restore terminal
    restore_tui()?;
    terminal.show_cursor()?;

    Ok(())
}

/// Overwrits the default panic hook by first
/// trying to restore our terminal
fn init_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Ignore errors, as we are already panicing
        let _ = restore_tui();
        original_hook(panic_info);
    }));
}

fn init_tui() -> io::Result<Terminal<impl Backend>> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

fn restore_tui() -> io::Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

async fn run<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<(), std::io::Error> {
    terminal.draw(|f| ui(f, app))?;
    app.fetch_config().await;
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            // Do not react to releases
            if key.kind == event::KeyEventKind::Release {
                continue;
            }

            let mut handled = true;
            // Only mode dependent keys
            match app.current_mode {
                CurrentMode::Insert => match key.code {
                    KeyCode::Esc => {
                        app.normal_mode();
                    }
                    _ => handled = false,
                },
                CurrentMode::Normal => match key.code {
                    KeyCode::Char('q') => {
                        return Ok(());
                    }
                    KeyCode::Char('i') => {
                        app.insert_mode();
                    }
                    _ => handled = false,
                },
            }

            if handled {
                continue;
            }
            // Keys depending on mode and screen
            match app.current_mode {
                CurrentMode::Normal => match app.current_screen {
                    CurrentScreen::Main => match key.code {
                        KeyCode::Char('j') => match app.current_focus {
                            CurrentFocus::Folder => app.select_folder_down(),
                            CurrentFocus::Peer => app.select_peer_down(),
                        },
                        KeyCode::Char('k') => match app.current_focus {
                            CurrentFocus::Folder => app.select_folder_up(),
                            CurrentFocus::Peer => app.select_peer_up(),
                        },
                        KeyCode::Enter => match app.current_focus {
                            CurrentFocus::Folder => app.edit_selected_folder(),
                            CurrentFocus::Peer => app.edit_selected_peer(),
                        },
                        KeyCode::Char('o') => match app.current_focus {
                            CurrentFocus::Folder => app.open_create_folder(),
                            CurrentFocus::Peer => app.open_create_peer(),
                        },
                        KeyCode::Tab => {
                            app.toggle_focus();
                        }
                        _ => {}
                    },
                    CurrentScreen::EditFolder(_) => match key.code {
                        KeyCode::Esc => app.discard(),
                        _ => {}
                    },
                    CurrentScreen::EditPeer(_) => match key.code {
                        KeyCode::Esc => app.discard(),
                        _ => {}
                    },
                    CurrentScreen::CreateFolder(ref mut folder_state) => match key.code {
                        KeyCode::Tab => folder_state.toggle_focus(),
                        KeyCode::Esc => app.discard(),
                        KeyCode::Char('j') => folder_state.toggle_focus(), // Toggle is ok for wrapping around
                        KeyCode::Char('k') => folder_state.toggle_focus(),
                        KeyCode::Enter => app.post_folder().await,
                        _ => {}
                    },
                    CurrentScreen::CreatePeer => match key.code {
                        KeyCode::Esc => app.discard(),
                        _ => {}
                    },

                    _ => {}
                },
                CurrentMode::Insert => match app.current_screen {
                    CurrentScreen::Main => {}
                    CurrentScreen::CreateFolder(ref mut folder_state) => match folder_state.focus {
                        CreateFolderFocus::Folder => match key.code {
                            KeyCode::Char(to_insert) => {
                                folder_state.path_input.enter_char(to_insert)
                            }
                            KeyCode::Backspace => {
                                folder_state.path_input.delete_char();
                            }
                            KeyCode::Enter | KeyCode::Tab => {
                                // Go to next field, if currently editing path
                                folder_state.toggle_focus();
                            }
                            _ => {}
                        },
                        CreateFolderFocus::Id => match key.code {
                            KeyCode::Char(to_insert) => folder_state.id_input.enter_char(to_insert),
                            KeyCode::Backspace => folder_state.id_input.delete_char(),
                            KeyCode::Enter => app.post_folder().await,
                            KeyCode::Tab => folder_state.toggle_focus(),
                            _ => {}
                        },
                    },
                    _ => {}
                },
            }
        }
    }
}
