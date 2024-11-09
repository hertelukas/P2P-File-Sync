use app::{App, CurrentScreen};
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

fn main() -> eyre::Result<()> {
    // Use info as default logging level
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    color_eyre::install()?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let mut app = App::new();
    let _ = run(&mut terminal, &mut app);

    //restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}

fn run<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<(), std::io::Error> {
    loop {
        terminal.draw(|f| ui(f, app))?;

        if let Event::Key(key) = event::read()? {
            // Do not react to releases
            if key.kind == event::KeyEventKind::Release {
                continue;
            }

            match app.current_mode {
                app::CurrentMode::Insert => match app.current_screen {
                    _ => match key.code {
                        KeyCode::Esc => {
                            app.normal_mode();
                        }
                        _ => {}
                    },
                },
                app::CurrentMode::Normal => match app.current_screen {
                    // Keys that have the same effect on each screen
                    _ => match key.code {
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
                },
            }
        }
    }
}
