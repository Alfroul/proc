use crossterm::event::{KeyCode, KeyEvent};

/// 搜索框状态。
///
/// v0.6.0 阶段 4：新增 `query_lower` 缓存字段。搜索框每次输入只重算
/// lowercase 一次（O(query.len())），后续 N 进程过滤全部走预计算的
/// `ProcessInfo::name_lower` + `query_lower.contains`，避免每按键 N 次
/// `to_lowercase` 分配。
pub struct SearchState {
    pub active: bool,
    pub query: String,
    /// v0.6.0 阶段 4：query 的 lowercase 缓存。`handle_input` / `clear` 维护。
    pub query_lower: String,
}

impl Default for SearchState {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            active: false,
            query: String::new(),
            query_lower: String::new(),
        }
    }

    /// Handle search input. Returns true if the key was consumed.
    pub fn handle_input(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.active = false;
                self.query.clear();
                self.query_lower.clear();
                true
            }
            KeyCode::Enter => {
                self.active = false;
                true
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.query_lower = self.query.to_lowercase();
                true
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                // 仅对新增字符做 lowercase 追加比整体重算更便宜，但整体重算在
                // query 长度 < 64 时差异可忽略；保持简单。
                self.query_lower = self.query.to_lowercase();
                true
            }
            _ => false,
        }
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.active
    }

    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// v0.6.0 阶段 4：lowercase query 借用。配合 `ProcessInfo::name_lower` 用。
    #[must_use]
    pub fn query_lower(&self) -> &str {
        &self.query_lower
    }

    pub fn clear(&mut self) {
        self.active = false;
        self.query.clear();
        self.query_lower.clear();
    }
}
