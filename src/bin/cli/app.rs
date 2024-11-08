//! Stores persistant data of the CLI binary

/// Used to switch the
pub enum CurrentScreen {
    Main,
}

pub struct App {
    pub current_screen: CurrentScreen,
}

impl App {
    pub fn new() -> App {
        App {
            current_screen: CurrentScreen::Main,
        }
    }
}
