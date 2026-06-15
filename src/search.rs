use crossterm::event::{KeyCode, KeyEvent};

pub struct SearchState {
    pub active: bool,
    pub query: String,
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
        }
    }

    /// Handle search input. Returns true if the key was consumed.
    pub fn handle_input(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.active = false;
                self.query.clear();
                true
            }
            KeyCode::Enter => {
                self.active = false;
                true
            }
            KeyCode::Backspace => {
                self.query.pop();
                true
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                true
            }
            _ => false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.query.clear();
    }
}
