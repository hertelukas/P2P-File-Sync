//! Stores persistant data of the CLI binary
use std::sync::{Arc, Mutex};

use p2p_file_sync::{Config, Peer, WatchedFolder};
use tokio::task;

/// Used to switch the screen
#[derive(Clone)]
pub enum CurrentScreen {
    Main,
    Loading,
    Error(String),
    EditFolder(WatchedFolder),
    EditPeer(Peer),
    CreateFolder,
    CreatePeer,
}

/// More or less vim modes
pub enum CurrentMode {
    Insert,
    Normal,
}

/// Which object we are currently editing
pub enum CurrentFocus {
    Folder,
    Peer,
}

pub struct App {
    pub current_screen: Arc<Mutex<CurrentScreen>>,
    pub current_mode: CurrentMode,
    pub current_focus: CurrentFocus,
    pub config: Arc<Mutex<Option<Config>>>,
    pub selected_folder: Option<usize>,
    pub selected_peer: Option<usize>,
}

impl App {
    pub fn new() -> App {
        App {
            current_screen: Arc::new(Mutex::new(CurrentScreen::Main)),
            current_mode: CurrentMode::Normal,
            current_focus: CurrentFocus::Folder,
            config: Arc::new(Mutex::new(None)),
            selected_folder: None,
            selected_peer: None,
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
            CurrentFocus::Folder => self.current_focus = CurrentFocus::Peer,
            CurrentFocus::Peer => self.current_focus = CurrentFocus::Folder,
        }
    }

    pub fn edit_selected_folder(&mut self) {
        // Do nothing, if we don't have a folder selected
        if let Some(selected_folder) = self.selected_folder {
            // First, we clone our WatchedFolder and drop the mutex
            let folder: WatchedFolder = {
                let config_lock = self.config.lock().unwrap();
                if let Some(ref config) = *config_lock {
                    config.paths()[selected_folder].clone()
                } else {
                    // Do nothing if we have no config
                    return;
                }
            };

            // Now, update our folder
            self.set_screen(CurrentScreen::EditFolder(folder));
        }
    }

    pub fn edit_selected_peer(&mut self) {
        // Do nothing, if we don't have a folder selected
        if let Some(selected_pper) = self.selected_peer {
            // First, we clone our WatchedFolder and drop the mutex
            let peer: Peer = {
                let config_lock = self.config.lock().unwrap();
                if let Some(ref config) = *config_lock {
                    config.peers()[selected_pper].clone()
                } else {
                    // Do nothing if we have no config
                    return;
                }
            };

            // Now, update our folder
            self.set_screen(CurrentScreen::EditPeer(peer));
        }
    }

    /// Selects the next folder downwards (so the next larger index)
    pub fn select_folder_down(&mut self) {
        let n = self.number_folders();

        // If no folders, set to None
        if n == 0 {
            self.selected_folder = None;
        } else {
            if let Some(current) = self.selected_folder {
                self.selected_folder = Some((current + 1) % n);
            } else {
                self.selected_folder = Some(0);
            }
        }
    }

    pub fn select_folder_up(&mut self) {
        let n = self.number_folders();

        // If no folders, set to None
        if n == 0 {
            self.selected_folder = None;
        } else {
            if let Some(current) = self.selected_folder {
                self.selected_folder = Some(((current + n) - 1) % n); // + n to avoid underflow
            } else {
                self.selected_folder = Some(n - 1);
            }
        }
    }

    pub fn select_peer_down(&mut self) {
        let n = self.number_peers();

        // If no folders, set to None
        if n == 0 {
            self.selected_peer = None;
        } else {
            if let Some(current) = self.selected_peer {
                self.selected_peer = Some((current + 1) % n);
            } else {
                self.selected_peer = Some(0);
            }
        }
    }

    pub fn select_peer_up(&mut self) {
        let n = self.number_peers();

        // If no folders, set to None
        if n == 0 {
            self.selected_peer = None;
        } else {
            if let Some(current) = self.selected_peer {
                self.selected_peer = Some(((current + n) - 1) % n); // + n to avoid underflow
            } else {
                self.selected_peer = Some(n - 1);
            }
        }
    }

    pub fn open_create_folder(&mut self) {
        self.set_screen(CurrentScreen::CreateFolder);
    }

    pub fn open_create_peer(&mut self) {
        self.set_screen(CurrentScreen::CreatePeer);
    }

    fn number_folders(&mut self) -> usize {
        let lock = self.config.lock().unwrap();
        if let Some(ref config) = *lock {
            config.paths().len()
        } else {
            0
        }
    }

    fn number_peers(&mut self) -> usize {
        let lock = self.config.lock().unwrap();
        if let Some(ref config) = *lock {
            config.peers().len()
        } else {
            0
        }
    }

    fn set_screen(&mut self, screen: CurrentScreen) {
        let mut screen_lock = self.current_screen.lock().unwrap();
        *screen_lock = screen;
    }

    pub fn fetch_config(&mut self) {
        self.set_screen(CurrentScreen::Loading);

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
