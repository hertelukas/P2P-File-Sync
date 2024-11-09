//! Stores persistant data of the CLI binary

/// Used to switch the screen
pub enum CurrentScreen {
    Main,
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
    pub current_screen: CurrentScreen,
    pub current_mode: CurrentMode,
    pub current_focus: CurrentFocus,
}

impl App {
    pub fn new() -> App {
        App {
            current_screen: CurrentScreen::Main,
            current_mode: CurrentMode::Normal,
            current_focus: CurrentFocus::File,
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
}
