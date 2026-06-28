use crossterm::event::{KeyCode, KeyEvent};

use crate::filter::FilterExpr;

/// 搜索框的语法模式。
///
/// v0.7.0 阶段 4：`Substring` 是 v0.6 唯一路径（保留 100% 行为兼容）；
/// `FilterExpr` 走 [`crate::filter::parse`] 解析为 AST。激活方式：搜索框第一字符
/// `:` → FilterExpr；`/` → Substring。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum QueryMode {
    /// v0.6 子串匹配：name.to_lowercase().contains(query_lower) || pid.contains(query)。
    #[default]
    Substring,
    /// v0.7 表达式过滤：FilterExpr::apply(process)。
    FilterExpr,
}

/// 搜索框状态。
///
/// v0.6.0 阶段 4：新增 `query_lower` 缓存字段。搜索框每次输入只重算
/// lowercase 一次（O(query.len())），后续 N 进程过滤全部走预计算的
/// `ProcessInfo::name_lower` + `query_lower.contains`，避免每按键 N 次
/// `to_lowercase` 分配。
///
/// v0.7.0 阶段 1 TD-9：`query_lower` 在全 ASCII 输入下走增量 push/pop
/// （O(1) 每按键）。非 ASCII 字符（如 `İ` → `i̇` 多字节大小写映射）触发
/// fallback 整体重算，由 `query_lower_ascii_sync` 标记区分。query 清空时
/// 标记重置，回到增量路径。
///
/// v0.7.0 阶段 4：加 `mode: QueryMode` + `filter_expr` / `filter_error`。
/// FilterExpr 模式下每次按键 re-parse；parse 失败保留上一次成功 AST，让
/// cached_sorted 继续按旧 AST 过滤，UI 显示错误。
pub struct SearchState {
    pub active: bool,
    pub query: String,
    /// v0.6.0 阶段 4：query 的 lowercase 缓存。`handle_input` / `clear` 维护。
    /// FilterExpr 模式下不再用于匹配，但保持同步以便切换回 Substring 时无需重算。
    pub query_lower: String,
    /// v0.7.0 阶段 1 TD-9：`query_lower` 是否与 `query` 同步走 ASCII 增量
    /// push/pop。非 ASCII 字符强制 false 走整体重算，避免多字节大小写映射
    /// 让 query / query_lower 字符数错位破坏 backspace 同步。
    query_lower_ascii_sync: bool,
    /// v0.7.0 阶段 4：当前语法模式。`/` 激活 → Substring；`:` 激活 → FilterExpr。
    pub mode: QueryMode,
    /// v0.7.0 阶段 4：FilterExpr 模式下最后一次成功 parse 的 AST。Substring 模式保持 None。
    /// parse 失败时保留上一次成功值（不重置为 None），让 cached_sorted 按旧 AST 继续过滤。
    pub filter_expr: Option<FilterExpr>,
    /// v0.7.0 阶段 4：FilterExpr 模式下若 parse 失败，存友好错误信息（渲染到 status_message）。
    pub filter_error: Option<String>,
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
            query_lower_ascii_sync: true,
            mode: QueryMode::Substring,
            filter_expr: None,
            filter_error: None,
        }
    }

    /// 切到 Substring 模式并激活。`/` 键入口。
    pub fn activate_substring(&mut self) {
        self.active = true;
        self.mode = QueryMode::Substring;
    }

    /// 切到 FilterExpr 模式并激活。`:` 键入口。清掉之前的 AST / 错误。
    /// query 应由调用方在按下 `:` 时清空（`:` 本身不进 query）。
    pub fn activate_filter_expr(&mut self) {
        self.active = true;
        self.mode = QueryMode::FilterExpr;
        self.filter_expr = None;
        self.filter_error = None;
    }

    /// Handle search input. Returns true if the key was consumed.
    pub fn handle_input(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => {
                self.reset_state();
                true
            }
            KeyCode::Enter => {
                // v0.6 行为：Enter 退出搜索框，但保留 query 让过滤结果继续生效。
                // 同样适用于 FilterExpr 模式：保留 AST 继续过滤。
                self.active = false;
                true
            }
            KeyCode::Backspace => {
                self.query.pop();
                if self.query_lower_ascii_sync {
                    // ASCII 路径：query_lower 与 query 字符一一对应，pop 一次同步。
                    self.query_lower.pop();
                } else {
                    // fallback 路径：query_lower 可能字符数 ≠ query（如 `İ`→`i̇`），
                    // 整体重算保证一致。
                    self.query_lower = self.query.to_lowercase();
                }
                // query 空了重置 sync 标记，让下一次输入回到增量路径。
                if self.query.is_empty() {
                    self.query_lower_ascii_sync = true;
                }
                if self.mode == QueryMode::FilterExpr {
                    self.reparse_filter();
                }
                true
            }
            KeyCode::Char(c) => {
                self.query.push(c);
                if self.query_lower_ascii_sync && c.is_ascii() {
                    // 增量：c 是 ASCII（含符号 / 数字），to_ascii_lowercase O(1)。
                    self.query_lower.push(c.to_ascii_lowercase());
                } else {
                    // fallback：非 ASCII（可能多字节大小写映射）或已脱同步，整体重算。
                    self.query_lower_ascii_sync = false;
                    self.query_lower = self.query.to_lowercase();
                }
                if self.mode == QueryMode::FilterExpr {
                    self.reparse_filter();
                }
                true
            }
            _ => false,
        }
    }

    /// FilterExpr 模式下用最新 query re-parse。空 query → 清 AST + 错误（视为无过滤）。
    /// parse 失败 → 保留上一次成功的 AST（不动 self.filter_expr），写错误信息。
    fn reparse_filter(&mut self) {
        if self.query.is_empty() {
            self.filter_expr = None;
            self.filter_error = None;
            return;
        }
        match crate::filter::parse(&self.query) {
            Ok(expr) => {
                self.filter_expr = Some(expr);
                self.filter_error = None;
            }
            Err(e) => {
                // 关键：保留上一次 self.filter_expr，让 cached_sorted 继续按旧 AST 过滤。
                self.filter_error = Some(format!("{e}"));
            }
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
        self.reset_state();
    }

    fn reset_state(&mut self) {
        self.active = false;
        self.query.clear();
        self.query_lower.clear();
        self.query_lower_ascii_sync = true;
        self.mode = QueryMode::Substring;
        self.filter_expr = None;
        self.filter_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn ascii_input_keeps_lower_in_sync() {
        let mut s = SearchState::new();
        for c in "Chrome".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        assert_eq!(s.query(), "Chrome");
        assert_eq!(s.query_lower(), "chrome");
    }

    #[test]
    fn ascii_backspace_pops_lower() {
        let mut s = SearchState::new();
        for c in "ABC".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        assert_eq!(s.query_lower(), "abc");
        s.handle_input(key(KeyCode::Backspace));
        assert_eq!(s.query(), "AB");
        assert_eq!(s.query_lower(), "ab");
        s.handle_input(key(KeyCode::Backspace));
        s.handle_input(key(KeyCode::Backspace));
        assert_eq!(s.query(), "");
        assert_eq!(s.query_lower(), "");
    }

    #[test]
    fn non_ascii_input_falls_back_correctly() {
        // `İ` (U+0130, Turkish dotted I) → lowercase `i̇` (U+0069 + U+0307)。
        // 多字节大小写映射，必须走 fallback 整体重算。
        let mut s = SearchState::new();
        s.handle_input(key(KeyCode::Char('İ')));
        assert_eq!(s.query(), "İ");
        assert_eq!(s.query_lower(), "i̇");
    }

    #[test]
    fn non_ascii_then_backspace_recalculates() {
        // 关键回归：若 query_lower 走增量 pop，query=`İ` / lower=`i̇`(2 chars)
        // pop 一次会让 lower 留下 combining dot（坏）。fallback 重算保证一致。
        let mut s = SearchState::new();
        s.handle_input(key(KeyCode::Char('İ')));
        s.handle_input(key(KeyCode::Backspace));
        assert_eq!(s.query(), "");
        assert_eq!(s.query_lower(), "");
    }

    #[test]
    fn ascii_after_non_ascii_still_uses_fallback() {
        // 一旦走 fallback，后续 ASCII 输入也整体重算（surgical：保持简单，
        // 不做 O(N) ASCII 重新检测）。query 清空后才回到增量路径。
        let mut s = SearchState::new();
        s.handle_input(key(KeyCode::Char('İ')));
        s.handle_input(key(KeyCode::Char('a')));
        assert_eq!(s.query(), "İa");
        assert_eq!(s.query_lower(), "i̇a");
    }

    #[test]
    fn ascii_after_clear_resumes_incremental() {
        // clear() 后 sync 标记重置，下一次 ASCII 输入回到增量路径。
        let mut s = SearchState::new();
        s.handle_input(key(KeyCode::Char('İ')));
        s.clear();
        s.handle_input(key(KeyCode::Char('A')));
        assert_eq!(s.query(), "A");
        assert_eq!(s.query_lower(), "a");
    }

    #[test]
    fn esc_resets_sync_flag() {
        let mut s = SearchState::new();
        s.handle_input(key(KeyCode::Char('İ')));
        s.handle_input(key(KeyCode::Esc));
        s.handle_input(key(KeyCode::Char('X')));
        assert_eq!(s.query(), "X");
        assert_eq!(s.query_lower(), "x");
    }

    #[test]
    fn ascii_digits_and_symbols_passthrough() {
        // 符号 / 数字 `to_ascii_lowercase` 返回自身，增量路径仍正确。
        let mut s = SearchState::new();
        for c in "abc-123_XYZ".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        assert_eq!(s.query(), "abc-123_XYZ");
        assert_eq!(s.query_lower(), "abc-123_xyz");
    }

    // --- v0.7 阶段 4：FilterExpr mode 测试 ---

    #[test]
    fn activate_filter_expr_sets_mode() {
        let mut s = SearchState::new();
        s.activate_filter_expr();
        assert_eq!(s.mode, QueryMode::FilterExpr);
        assert!(s.is_active());
        assert!(s.filter_expr.is_none());
        assert!(s.filter_error.is_none());
    }

    #[test]
    fn filter_expr_parses_on_char_push() {
        let mut s = SearchState::new();
        s.activate_filter_expr();
        for c in "cpu > 5".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        assert_eq!(s.query, "cpu > 5");
        assert!(s.filter_error.is_none());
        assert!(s.filter_expr.is_some());
    }

    #[test]
    fn filter_expr_error_kept_on_bad_input() {
        let mut s = SearchState::new();
        s.activate_filter_expr();
        // 故意输入非法：`cpu >>`，parse 应失败。
        for c in "cpu >>".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        assert!(s.filter_error.is_some(), "expected error after bad input");
        // 第一次失败时无先前 AST，filter_expr 保持 None。
        assert!(s.filter_expr.is_none());
    }

    #[test]
    fn filter_expr_keeps_prev_ast_on_parse_error() {
        let mut s = SearchState::new();
        s.activate_filter_expr();
        // 先打一个合法表达式
        for c in "cpu > 5".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        assert!(s.filter_expr.is_some());

        // 接着打坏字符：query = `cpu > 5)` → 右括号多余 → parse 错
        s.handle_input(key(KeyCode::Char(')')));
        assert!(s.filter_error.is_some(), "expected error after stray )");
        // 关键：保留上一次成功 AST（filter_expr 仍 Some），让 cached_sorted 继续过滤。
        // FilterExpr 内含 regex::Regex 无法 PartialEq，靠 is_some + apply 间接验证。
        let expr = s
            .filter_expr
            .as_ref()
            .expect("previous AST must be retained on parse error");
        let p = crate::collect::ProcessInfo::default();
        // cpu=0 不满足 `cpu > 5`，apply 应为 false。说明 expr 仍是 cpu>5 而非被清空。
        let ctx = crate::filter::EvalCtx {
            process: &p,
            security_score: None,
        };
        assert!(!expr.apply(&ctx));
    }

    #[test]
    fn esc_resets_filter_mode() {
        let mut s = SearchState::new();
        s.activate_filter_expr();
        for c in "cpu > 5".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        s.handle_input(key(KeyCode::Esc));
        assert_eq!(s.mode, QueryMode::Substring);
        assert!(s.query.is_empty());
        assert!(s.filter_expr.is_none());
        assert!(s.filter_error.is_none());
    }

    #[test]
    fn empty_filter_expr_clears_ast() {
        let mut s = SearchState::new();
        s.activate_filter_expr();
        for c in "cpu > 5".chars() {
            s.handle_input(key(KeyCode::Char(c)));
        }
        assert!(s.filter_expr.is_some());
        // 全部 backspace 掉
        for _ in 0.."cpu > 5".len() {
            s.handle_input(key(KeyCode::Backspace));
        }
        assert!(s.query.is_empty());
        assert!(s.filter_expr.is_none());
        assert!(s.filter_error.is_none());
    }
}
