//! Stores persistant data of the CLI binary
use std::sync::{Arc, Mutex};

use p2p_file_sync::Config;
use tokio::task;

/// Used to switch the screen
pub enum CurrentScreen {
    Main,
    Loading,
    Error(String),
}

/// More or less vim modes
pub enum CurrentMode {
    Insert,
    Normal,
}

/// Which object we are currently editing
pub enum CurrentFocus {
    File,
    Peer,
}

pub struct App {
    pub current_screen: Arc<Mutex<CurrentScreen>>,
    pub current_mode: CurrentMode,
    pub current_focus: CurrentFocus,
    pub config: Arc<Mutex<Option<Config>>>,
}

impl App {
    pub fn new() -> App {
        App {
            current_screen: Arc::new(Mutex::new(CurrentScreen::Main)),
            current_mode: CurrentMode::Normal,
            current_focus: CurrentFocus::File,
            config: Arc::new(Mutex::new(None)),
        }
    }

    pub fn insert_mode(&mut self) {
        self.current_mode = CurrentMode::Insert;
    }

    pub fn normal_mode(&mut self) {
        self.current_mode = CurrentMode::Normal;
    }

    pub fn toggle_focus(&mut self) {
        match &self.current_focus {
            CurrentFocus::File => self.current_focus = CurrentFocus::Peer,
            CurrentFocus::Peer => self.current_focus = CurrentFocus::File,
        }
    }

    pub fn fetch_config(&mut self) {
        {
            let mut screen_lock = self.current_screen.lock().unwrap();
            *screen_lock = CurrentScreen::Loading;
        }

        let config_handle = Arc::clone(&self.config);
        let screen_handle = Arc::clone(&self.current_screen);

        task::spawn(async move {
            match reqwest::get("http://127.0.0.1:3617").await {
                Ok(resp) => match resp.json::<Config>().await {
                    Ok(config) => {
                        let mut config_lock = config_handle.lock().unwrap();
                        *config_lock = Some(config);
                        drop(config_lock);
                        let mut screen_lock = screen_handle.lock().unwrap();
                        *screen_lock = CurrentScreen::Main;
                    }
                    Err(e) => {
                        let mut screen_lock = screen_handle.lock().unwrap();
                        *screen_lock = CurrentScreen::Error(format!("Could not parse config: {e}"));
                    }
                },
                Err(e) => {
                    let mut screen_lock = screen_handle.lock().unwrap();
                    *screen_lock = CurrentScreen::Error(format!("Server unreachable: {e}"));
                }
            }
        });
    }
}
