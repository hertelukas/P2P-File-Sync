//! Stores persistant data of the CLI binary

use p2p_file_sync::{Config, Peer, WatchedFolder};
use reqwest::Client;

/// Used to switch the screen
pub enum CurrentScreen {
    Main,
    Loading,
    Error(String),
    EditFolder(EditFolderState),
    EditPeer(Peer),
    CreateFolder(EditFolderState),
    CreatePeer,
    DeleteFolder(WatchedFolder),
}

#[derive(Default)]
pub struct EditFolderState {
    pub path_input: TextBox,
    pub id_input: TextBox,
    pub focus: EditFolderFocus,
}

impl EditFolderState {
    pub fn toggle_focus(&mut self) {
        match &self.focus {
            EditFolderFocus::Folder => self.focus = EditFolderFocus::Id,
            EditFolderFocus::Id => self.focus = EditFolderFocus::Folder,
        }
    }
}

#[derive(Debug)]
pub struct FolderError;

impl TryInto<WatchedFolder> for &EditFolderState {
    type Error = FolderError;

    fn try_into(self) -> Result<WatchedFolder, Self::Error> {
        match (&self.id_input).try_into() as Result<u32, _> {
            Ok(id) => Ok(WatchedFolder::new_full(id, &self.path_input.text)),
            Err(_) => Err(FolderError),
        }
    }
}

impl From<WatchedFolder> for EditFolderState {
    fn from(folder: WatchedFolder) -> Self {
        EditFolderState {
            path_input: TextBox {
                text: folder.path.to_string_lossy().to_string(),
                index: folder.path.to_string_lossy().len(),
            },
            id_input: TextBox {
                text: format!("{:#x}", folder.id),
                index: format!("{:#x}", folder.id).len(),
            },
            focus: EditFolderFocus::Folder,
        }
    }
}

#[derive(Default)]
pub enum EditFolderFocus {
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
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    ParseError(#[from] std::num::ParseIntError),
}

impl TryInto<u32> for &TextBox {
    type Error = Error;

    fn try_into(self) -> Result<u32, Self::Error> {
        if self.text.starts_with("0x") {
            u32::from_str_radix(&self.text[2..], 16).map_err(Error::from)
        } else {
            self.text.parse().map_err(Error::from)
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
    address: String,
}

impl App {
    pub fn new() -> App {
        let port = std::env::var("PORT").unwrap_or_else(|_| "3617".to_string());
        App {
            current_screen: CurrentScreen::Loading,
            current_mode: CurrentMode::Normal,
            current_focus: CurrentFocus::Folder,
            config: None,
            selected_folder: None,
            selected_peer: None,
            client: Client::new(),
            address: format!("http://127.0.0.1:{}", port),
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
                    config.paths[selected_folder].clone()
                } else {
                    // Do nothing if we have no config
                    return;
                }
            };

            // Now, update our folder
            self.current_screen = CurrentScreen::EditFolder(folder.into());
        }
    }

    pub fn edit_selected_peer(&mut self) {
        // Do nothing, if we don't have a folder selected
        if let Some(selected_pper) = self.selected_peer {
            // First, we clone our WatchedFolder and drop the mutex
            let peer: Peer = {
                if let Some(ref config) = self.config {
                    config.peers[selected_pper].clone()
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
        self.current_screen = CurrentScreen::CreateFolder(EditFolderState::default());
    }

    pub fn open_create_peer(&mut self) {
        self.current_screen = CurrentScreen::CreatePeer;
    }

    pub fn open_delete_folder(&mut self) {
        // Do nothing, if we don't have a folder selected
        if let Some(selected_folder) = self.selected_folder {
            // First, we clone our WatchedFolder and drop the mutex
            let folder: WatchedFolder = {
                if let Some(ref config) = self.config {
                    config.paths[selected_folder].clone()
                } else {
                    // Do nothing if we have no config
                    return;
                }
            };

            // Now, update our folder
            self.current_screen = CurrentScreen::DeleteFolder(folder.into());
        }
    }

    pub fn discard(&mut self) {
        self.current_screen = CurrentScreen::Main;
    }

    fn number_folders(&mut self) -> usize {
        if let Some(ref config) = self.config {
            config.paths.len()
        } else {
            0
        }
    }

    fn number_peers(&mut self) -> usize {
        if let Some(ref config) = self.config {
            config.peers.len()
        } else {
            0
        }
    }

    pub async fn put_folder(&mut self) {
        if let CurrentScreen::EditFolder(folder) = &self.current_screen {
            if let Ok(folder) = folder.try_into() as Result<WatchedFolder, _> {
                match self
                    .client
                    .put(format!("{}/folder", self.address))
                    .json(&folder)
                    .send()
                    .await
                {
                    Ok(_) => {
                        // Update our own folder
                        if let Some(ref mut config) = self.config {
                            config.update_folder_sync(folder, false).unwrap();
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
                    CurrentScreen::Error(format!("Failed to create valid folder"));
            }
        } else {
            self.current_screen = CurrentScreen::Error(
                "Can only update folder through the \"Edit Folder\" prompt".to_string(),
            )
        }
    }

    pub async fn delete_folder(&mut self) {
        if let CurrentScreen::DeleteFolder(folder) = &self.current_screen {
            match self
                .client
                .delete(format!("{}/folder", self.address))
                .json(&folder)
                .send()
                .await
            {
                Ok(_) => {
                    // Delete our folder from our local state
                    if let Some(ref mut config) = self.config {
                        // Never fails, as we do not store
                        config.delete_folder_sync(folder.id, false).unwrap();
                    }
                    self.current_screen = CurrentScreen::Main;
                    self.current_mode = CurrentMode::Normal;
                }
                Err(e) => {
                    self.current_screen = CurrentScreen::Error(format!("Server unreachable: {e}"))
                }
            }
        } else {
            self.current_screen = CurrentScreen::Error(
                "Can only delete folder through the \"Delete Folder\" prompt".to_string(),
            )
        }
    }

    pub async fn post_folder(&mut self) {
        if let CurrentScreen::CreateFolder(folder_input) = &self.current_screen {
            if let Ok(folder) = folder_input.try_into() as Result<WatchedFolder, _> {
                match self
                    .client
                    .post(format!("{}/folder", self.address))
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
        match self.client.get(self.address.clone()).send().await {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_textbox_enter() {
        let mut textbox = TextBox::default();
        assert_eq!(textbox.text, "");
        assert_eq!(textbox.index, 0);

        textbox.enter_char('a');
        textbox.enter_char('b');
        textbox.enter_char('c');

        assert_eq!(textbox.text, "abc");
        assert_eq!(textbox.index, 3);
    }

    #[test]
    fn test_textbox_delete() {
        let mut textbox = TextBox::default();
        assert_eq!(textbox.text, "");
        assert_eq!(textbox.index, 0);

        textbox.enter_char('a');
        textbox.enter_char('b');
        textbox.enter_char('c');

        assert_eq!(textbox.text, "abc");
        assert_eq!(textbox.index, 3);

        textbox.delete_char();
        textbox.delete_char();
        assert_eq!(textbox.text, "a");
        assert_eq!(textbox.index, 1);
    }

    #[test]
    fn test_textbox_delete_too_far() {
        let mut textbox = TextBox::default();
        assert_eq!(textbox.text, "");
        assert_eq!(textbox.index, 0);

        textbox.enter_char('a');
        textbox.enter_char('b');
        textbox.enter_char('c');

        assert_eq!(textbox.text, "abc");
        assert_eq!(textbox.index, 3);

        textbox.delete_char();
        textbox.delete_char();
        textbox.delete_char();
        textbox.delete_char(); // Make sure that we can handle this
        assert_eq!(textbox.text, "");
        assert_eq!(textbox.index, 0);
    }

    #[test]
    fn test_textbox_try_into() {
        let mut textbox = TextBox::default();

        textbox.enter_char('1');
        textbox.enter_char('2');

        let i = ((&textbox).try_into() as Result<u32, _>).unwrap();

        assert_eq!(i, 12);
    }

    #[test]
    fn test_textbox_try_into_hex() {
        let mut textbox = TextBox::default();

        textbox.enter_char('0');
        textbox.enter_char('x');
        textbox.enter_char('1');
        textbox.enter_char('2');

        let i = ((&textbox).try_into() as Result<u32, _>).unwrap();

        assert_eq!(i, 18);
    }

    #[test]
    #[should_panic]
    fn test_textbox_try_into_fails() {
        let mut textbox = TextBox::default();

        textbox.enter_char('0');
        textbox.enter_char('x');

        let _ = ((&textbox).try_into() as Result<u32, _>).unwrap();
    }

    #[test]
    fn test_create_folder_state() {
        let mut folder_state = EditFolderState::default();

        assert!(matches!(folder_state.focus, EditFolderFocus::Folder));
        folder_state.toggle_focus();
        assert!(matches!(folder_state.focus, EditFolderFocus::Id));
    }

    #[test]
    fn test_create_folder_into() {
        let mut folder_state = EditFolderState::default();

        folder_state.path_input.enter_char('/');
        folder_state.path_input.enter_char('f');
        folder_state.path_input.enter_char('o');
        folder_state.path_input.enter_char('o');

        folder_state.id_input.enter_char('8');
        folder_state.id_input.enter_char('2');

        match (&folder_state).try_into() as Result<WatchedFolder, _> {
            Ok(f) => {
                assert_eq!(f.id, 82);
                assert_eq!(f.path, std::path::PathBuf::from("/foo"));
            }
            Err(_) => panic!("Failed into watched folder"),
        }
    }
}
