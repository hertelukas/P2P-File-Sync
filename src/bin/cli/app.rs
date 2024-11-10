//! Stores persistant data of the CLI binary

use p2p_file_sync::{Config, Peer, WatchedFolder};
use reqwest::Client;

/// Used to switch the screen
pub enum CurrentScreen {
    Main,
    Loading,
    Error(String),
    EditFolder(WatchedFolder),
    EditPeer(Peer),
    CreateFolder(CreateFolderState),
    CreatePeer,
}

#[derive(Default)]
pub struct CreateFolderState {
    pub path_input: TextBox,
    pub id_input: TextBox,
    pub focus: CreateFolderFocus,
}

impl CreateFolderState {
    pub fn toggle_focus(&mut self) {
        match &self.focus {
            CreateFolderFocus::Folder => self.focus = CreateFolderFocus::Id,
            CreateFolderFocus::Id => self.focus = CreateFolderFocus::Folder,
        }
    }
}

#[derive(Debug)]
pub struct FolderError;

impl TryInto<WatchedFolder> for &CreateFolderState {
    type Error = FolderError;

    fn try_into(self) -> Result<WatchedFolder, Self::Error> {
        match self.id_input.as_int() {
            Some(id) => Ok(WatchedFolder::new_full(id, &self.path_input.text)),
            None => Err(FolderError),
        }
    }
}

#[derive(Default)]
pub enum CreateFolderFocus {
    #[default]
    Folder,
    Id,
}

#[derive(Default)]
pub struct TextBox {
    pub text: String,
    pub index: usize,
}

// This impl is heavily inspired (copied) by https://ratatui.rs/examples/apps/user_input/
impl TextBox {
    fn move_cursor_left(&mut self) {
        let cursor_moved_left = self.index.saturating_sub(1);
        self.index = self.clamp_cursor(cursor_moved_left);
    }

    fn move_cursor_right(&mut self) {
        let cursor_moved_right = self.index.saturating_add(1);
        self.index = self.clamp_cursor(cursor_moved_right);
    }

    pub fn enter_char(&mut self, new_char: char) {
        let index = self.byte_index();
        self.text.insert(index, new_char);
        self.move_cursor_right();
    }

    /// Returns the byte index based on the character position.
    ///
    /// Since each character in a string can be contain multiple bytes, it's necessary to calculate
    /// the byte index based on the index of the character.
    fn byte_index(&self) -> usize {
        self.text
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.index)
            .unwrap_or(self.text.len())
    }

    pub fn delete_char(&mut self) {
        let is_not_cursor_leftmost = self.index != 0;
        if is_not_cursor_leftmost {
            // Method "remove" is not used on the saved text for deleting the selected char.
            // Reason: Using remove on String works on bytes instead of the chars.
            // Using remove would require special care because of char boundaries.

            let current_index = self.index;
            let from_left_to_current_index = current_index - 1;

            // Getting all characters before the selected character.
            let before_char_to_delete = self.text.chars().take(from_left_to_current_index);
            // Getting all characters after selected character.
            let after_char_to_delete = self.text.chars().skip(current_index);

            // Put all characters together except the selected one.
            // By leaving the selected one out, it is forgotten and therefore deleted.
            self.text = before_char_to_delete.chain(after_char_to_delete).collect();
            self.move_cursor_left();
        }
    }

    fn clamp_cursor(&self, new_cursor_pos: usize) -> usize {
        new_cursor_pos.clamp(0, self.text.chars().count())
    }

    pub fn as_int(&self) -> Option<u32> {
        if self.text.starts_with("0x") {
            match u32::from_str_radix(&self.text[2..], 16) {
                Ok(val) => Some(val),
                Err(_) => None,
            }
        } else {
            match self.text.parse() {
                Ok(val) => Some(val),
                Err(_) => None,
            }
        }
    }
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
    pub current_screen: CurrentScreen,
    pub current_mode: CurrentMode,
    pub current_focus: CurrentFocus,
    pub config: Option<Config>,
    pub selected_folder: Option<usize>,
    pub selected_peer: Option<usize>,
    pub client: Client,
}

impl App {
    pub fn new() -> App {
        App {
            current_screen: CurrentScreen::Loading,
            current_mode: CurrentMode::Normal,
            current_focus: CurrentFocus::Folder,
            config: None,
            selected_folder: None,
            selected_peer: None,
            client: Client::new(),
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
                if let Some(ref config) = self.config {
                    config.paths()[selected_folder].clone()
                } else {
                    // Do nothing if we have no config
                    return;
                }
            };

            // Now, update our folder
            self.current_screen = CurrentScreen::EditFolder(folder);
        }
    }

    pub fn edit_selected_peer(&mut self) {
        // Do nothing, if we don't have a folder selected
        if let Some(selected_pper) = self.selected_peer {
            // First, we clone our WatchedFolder and drop the mutex
            let peer: Peer = {
                if let Some(ref config) = self.config {
                    config.peers()[selected_pper].clone()
                } else {
                    // Do nothing if we have no config
                    return;
                }
            };

            // Now, update our folder
            self.current_screen = CurrentScreen::EditPeer(peer);
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
        self.current_screen = CurrentScreen::CreateFolder(CreateFolderState::default());
    }

    pub fn open_create_peer(&mut self) {
        self.current_screen = CurrentScreen::CreatePeer;
    }

    pub fn discard(&mut self) {
        self.current_screen = CurrentScreen::Main;
    }

    fn number_folders(&mut self) -> usize {
        if let Some(ref config) = self.config {
            config.paths().len()
        } else {
            0
        }
    }

    fn number_peers(&mut self) -> usize {
        if let Some(ref config) = self.config {
            config.peers().len()
        } else {
            0
        }
    }

    pub async fn post_folder(&mut self) {
        if let CurrentScreen::CreateFolder(folder_input) = &self.current_screen {
            if let Ok(folder) = folder_input.try_into() as Result<WatchedFolder, _> {
                match self
                    .client
                    .post("http://127.0.0.1:3617/folder")
                    .json(&folder)
                    .send()
                    .await
                {
                    Ok(_) => {
                        // Add the folder to our state only if we get success back
                        if let Some(ref mut config) = self.config {
                            // This should never fail, as only storing could fail - what we are not doing
                            config.add_folder(folder.clone(), false).await.unwrap();
                        }
                        self.current_screen = CurrentScreen::Main;
                        self.current_mode = CurrentMode::Normal;
                    }
                    Err(e) => {
                        self.current_screen =
                            CurrentScreen::Error(format!("Server unreachable: {e}"))
                    }
                }
            } else {
                self.current_screen =
                    CurrentScreen::Error("Failed to create valid folder".to_string());
            }
        } else {
            self.current_screen = CurrentScreen::Error(
                "Can only create folder through the \"Create Folder\" prompt".to_string(),
            );
        }
    }

    pub async fn fetch_config(&mut self) {
        match self.client.get("http://127.0.0.1:3617").send().await {
            Ok(resp) => match resp.json::<Config>().await {
                Ok(config) => {
                    self.config = Some(config);
                    self.current_screen = CurrentScreen::Main;
                }
                Err(e) => {
                    self.current_screen =
                        CurrentScreen::Error(format!("Could not parse config: {e}"));
                }
            },
            Err(e) => {
                self.current_screen = CurrentScreen::Error(format!("Server unreachable: {e}"));
            }
        }
    }
}
