use app::App;
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
    let _ = run(&mut terminal, &mut app);

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

fn run<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<(), std::io::Error> {
    app.fetch_config();
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            // Do not react to releases
            if key.kind == event::KeyEventKind::Release {
                continue;
            }

            // Move the current screen outside of the lock
            let current_screen = {
                let lock = app.current_screen.lock().unwrap();
                (*lock).clone()
            };

            // Only mode dependent keys
            match app.current_mode {
                app::CurrentMode::Insert => match key.code {
                    KeyCode::Esc => {
                        app.normal_mode();
                    }
                    _ => {}
                },
                app::CurrentMode::Normal => match key.code {
                    KeyCode::Tab => {
                        app.toggle_focus();
                    }
                    KeyCode::Char('q') => {
                        return Ok(());
                    }
                    KeyCode::Char('i') => {
                        app.insert_mode();
                    }
                    _ => {}
                },
            }

            // Keys depending on mode and screen
            match app.current_mode {
                app::CurrentMode::Normal => match current_screen {
                    app::CurrentScreen::Main => match key.code {
                        KeyCode::Char('j') => match app.current_focus {
                            app::CurrentFocus::Folder => app.select_folder_down(),
                            app::CurrentFocus::Peer => app.select_peer_down(),
                        },
                        KeyCode::Char('k') => match app.current_focus {
                            app::CurrentFocus::Folder => app.select_folder_up(),
                            app::CurrentFocus::Peer => app.select_peer_up(),
                        },
                        KeyCode::Enter => match app.current_focus {
                            app::CurrentFocus::Folder => app.edit_selected_folder(),
                            app::CurrentFocus::Peer => app.edit_selected_peer(),
                        },
                        _ => {}
                    },
                    _ => {}
                },
                app::CurrentMode::Insert => match current_screen {
                    app::CurrentScreen::Main => {}
                    // Keys that have the same effect on each screen
                    _ => {}
                },
            }
        }
    }
}
