//! v0.14 阶段 3：录屏时间轴搜索。FilterExpr 扩 FrameField 维度，命中帧在
//! timeline 高亮 + n/N 跳转。
//!
//! 设计原则（与 stage 2 书签同款）：
//! - parse 失败保留上一次成功 AST（让 UI 在用户输入过程中继续过滤）
//! - substring 模式（无 `:` 前缀）走 [`crate::filter::build_frame_substring_expr`]
//!   构造 `name =~ /<input>/i` 表达式（regex 元字符 escape）
//! - 长录屏遍历可能慢（30 min × 30 FPS × 165 µs = ~9 秒），仅在 input 变化时
//!   调一次 [`ReplaySearch::recompute_matches`]；n/N 跳转只读 `matches` 列表

use crate::filter::{self, FilterExpr, FrameEvalCtx, ParseError, parse_frame};
use crate::record::UiFrame;

/// 搜索状态机。input 为空 = 未激活；非空 = 激活中。
///
/// 字段 `expr` 是「最后一次成功 parse 的 AST」，与 input 解耦——input 出错时
/// 保留 expr 让 UI 继续过滤（既有 FilterExpr UX 同款契约）。
#[derive(Debug, Clone)]
pub struct ReplaySearch {
    /// 用户输入的原始文本。`:` 前缀切到 FilterExpr 模式（与 List/Tree/Flow 视图同款）。
    pub input: String,
    /// 最后一次成功 parse 的 AST。input 出错时保留（让 UI 继续过滤）。
    pub expr: Option<FilterExpr>,
    /// 最近一次 parse 错误（input 不为空但 parse 失败时设）。让 UI 在 timeline 显示。
    pub error: Option<ParseError>,
    /// 命中帧索引列表（升序）。`recompute_matches` 一次性填充，n/N 跳转只读列表。
    pub matches: Vec<usize>,
    /// 当前 cursor 在 matches 中的索引（n/N 移动）。
    pub cursor: usize,
}

impl Default for ReplaySearch {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplaySearch {
    #[must_use]
    pub fn new() -> Self {
        Self {
            input: String::new(),
            expr: None,
            error: None,
            matches: Vec::new(),
            cursor: 0,
        }
    }

    /// 搜索是否激活（input 非空 或 expr 持有 AST）。
    #[must_use]
    pub fn is_active(&self) -> bool {
        !self.input.is_empty() || self.expr.is_some()
    }

    /// 用户输入字符 → push + 重新 parse + clear matches（让 recompute 在下次
    /// 调用时重算；不在 push_char 内调 recompute 是因为 frame_at 通常需 player
    /// 引用，调用方按节奏触发）。
    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
        self.reparse();
        self.matches.clear();
        self.cursor = 0;
    }

    pub fn pop_char(&mut self) {
        self.input.pop();
        self.reparse();
        self.matches.clear();
        self.cursor = 0;
    }

    /// 重置整个搜索（Esc 长按或调用方决定）。
    pub fn reset(&mut self) {
        self.input.clear();
        self.expr = None;
        self.error = None;
        self.matches.clear();
        self.cursor = 0;
    }

    /// 重新 parse input。失败保留上一次 expr（让 UI 继续过滤），error 显示给用户。
    fn reparse(&mut self) {
        if self.input.is_empty() {
            self.expr = None;
            self.error = None;
            return;
        }
        // `:` 前缀 → FilterExpr 模式；否则 substring 模式（构造 name =~ /input/i）
        let result = if let Some(stripped) = self.input.strip_prefix(':') {
            parse_frame(stripped)
        } else {
            filter::build_frame_substring_expr(&self.input)
        };
        match result {
            Ok(expr) => {
                self.expr = Some(expr);
                self.error = None;
            }
            Err(e) => {
                self.error = Some(e);
                // 保留 self.expr — UI 继续按上次成功 AST 过滤
            }
        }
    }

    /// 重新计算命中帧列表。仅在 input / expr 变化时调用一次；n/N 跳转只读 matches。
    ///
    /// `frame_at(idx) -> Option<UiFrame>` 让调用方注入 player（按需 seek + deserialize）。
    /// 长录屏遍历成本 = N × 165 µs（@ 1000 进程），30 min × 30 FPS = 54000 frames
    /// ≈ 9 秒——用户可感延迟；调用方应在 status_message 提示「正在搜索 N 帧…」。
    pub fn recompute_matches<F>(&mut self, total: usize, mut frame_at: F)
    where
        F: FnMut(usize) -> Option<UiFrame>,
    {
        self.matches.clear();
        self.cursor = 0;
        let Some(expr) = self.expr.as_ref() else {
            return;
        };
        for idx in 0..total {
            if let Some(frame) = frame_at(idx) {
                let ctx = FrameEvalCtx { frame: &frame };
                if expr.apply_frame(&ctx) {
                    self.matches.push(idx);
                }
            }
        }
    }

    /// n 键：跳转到下一命中（cursor + 1，clamp 在末尾）。返回目标帧 idx。
    #[must_use]
    pub fn next_match(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        self.cursor = (self.cursor + 1).min(self.matches.len() - 1);
        Some(self.matches[self.cursor])
    }

    /// N 键：跳转到上一命中（cursor - 1，clamp 在起点）。
    #[must_use]
    pub fn prev_match(&mut self) -> Option<usize> {
        if self.matches.is_empty() {
            return None;
        }
        self.cursor = self.cursor.saturating_sub(1);
        Some(self.matches[self.cursor])
    }

    /// 当前 cursor 处的帧 idx（n/N 跳转后的目标）。
    #[must_use]
    pub fn current_match(&self) -> Option<usize> {
        self.matches.get(self.cursor).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::frame::{
        FrameAnomaly, FrameConnectionDiff, FrameNav, FrameProcess, UiFrame,
    };

    fn make_frame(idx: usize, cpu: f32, mem: u64, names: &[&str], sev: &str) -> UiFrame {
        let processes = names
            .iter()
            .map(|n| FrameProcess {
                pid: 1,
                name: (*n).to_string(),
                cpu: 0.0,
                memory: 0,
                disk_read: 0,
                disk_write: 0,
            })
            .collect();
        let anomalies = if sev.is_empty() {
            Vec::new()
        } else {
            vec![FrameAnomaly {
                rule_id: "test".to_string(),
                severity: sev.to_string(),
                title: "test anomaly".to_string(),
                detail: String::new(),
                affected_pid: None,
                affected_ip: None,
            }]
        };
        UiFrame {
            timestamp: 1_000_000 + idx as u64,
            mode: "ProcessList".to_string(),
            status_message: None,
            cpu_usage: cpu,
            memory_used: mem,
            memory_total: 8 * 1024 * 1024 * 1024,
            net_down: 0,
            net_up: 0,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            processes,
            search_query: String::new(),
            sort_field: "Cpu".to_string(),
            process_view_mode: 0,
            tree_nodes: Vec::new(),
            port_entries: Vec::new(),
            port_view_mode: 0,
            port_process_groups: Vec::new(),
            port_remote_groups: Vec::new(),
            connection_diff: FrameConnectionDiff::default(),
            anomalies,
            usb_devices: Vec::new(),
            usb_locks: Vec::new(),
            monitors: Vec::new(),
            docker_containers: Vec::new(),
            docker_events: Vec::new(),
            ops: Vec::new(),
            nav: FrameNav::default(),
        }
    }

    #[test]
    fn new_search_is_inactive() {
        let s = ReplaySearch::new();
        assert!(!s.is_active());
        assert!(s.expr.is_none());
        assert!(s.error.is_none());
        assert!(s.matches.is_empty());
    }

    #[test]
    fn push_char_activates_and_parses() {
        let mut s = ReplaySearch::new();
        s.push_char(':');
        s.push_char('c');
        s.push_char('p');
        s.push_char('u');
        s.push_char(' ');
        s.push_char('>');
        s.push_char(' ');
        s.push_char('5');
        assert!(s.is_active());
        // input ":cpu > 5" 应 parse 成功
        assert!(s.expr.is_some());
        assert!(s.error.is_none());
    }

    #[test]
    fn substring_input_builds_regex_expr() {
        let mut s = ReplaySearch::new();
        // 无 `:` 前缀 → substring 模式（构造 name =~ /input/i）
        for c in "chrome".chars() {
            s.push_char(c);
        }
        assert!(s.expr.is_some());
        assert!(s.error.is_none());
        let expr = s.expr.as_ref().unwrap();
        assert!(expr.contains_frame_field());
    }

    #[test]
    fn parse_error_keeps_last_expr() {
        let mut s = ReplaySearch::new();
        // 成功 parse ":cpu > 5"
        for c in ":cpu > 5".chars() {
            s.push_char(c);
        }
        assert!(s.expr.is_some());

        // 错误输入 ":cpu >" → parse 失败 → 保留 expr_before
        s.push_char(' '); // 在 "5" 后加空格仍 OK
        s.pop_char();
        s.pop_char(); // 撤销 "5" → ":cpu >"
        assert!(s.expr.is_some(), "expr 应保留");
        assert!(s.error.is_some(), "error 应被设置");
    }

    #[test]
    fn recompute_matches_finds_cpu_high_frames() {
        let mut s = ReplaySearch::new();
        for c in ":cpu > 50".chars() {
            s.push_char(c);
        }
        let frames = [
            make_frame(0, 10.0, 0, &[], ""), // 不命中
            make_frame(1, 80.0, 0, &[], ""), // 命中
            make_frame(2, 30.0, 0, &[], ""), // 不命中
            make_frame(3, 95.0, 0, &[], ""), // 命中
        ];
        s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
        assert_eq!(s.matches, vec![1, 3]);
    }

    #[test]
    fn recompute_matches_name_in_processes() {
        let mut s = ReplaySearch::new();
        for c in ":name =~ /chrome/i".chars() {
            s.push_char(c);
        }
        let frames = [
            make_frame(0, 0.0, 0, &["explorer"], ""),
            make_frame(1, 0.0, 0, &["chrome.exe", "svchost"], ""), // 命中
            make_frame(2, 0.0, 0, &["firefox"], ""),
            make_frame(3, 0.0, 0, &["chrome"], ""), // 命中
        ];
        s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
        assert_eq!(s.matches, vec![1, 3]);
    }

    #[test]
    fn recompute_matches_anomaly_severity() {
        let mut s = ReplaySearch::new();
        for c in ":anomaly.severity = critical".chars() {
            s.push_char(c);
        }
        let frames = [
            make_frame(0, 0.0, 0, &[], "info"),
            make_frame(1, 0.0, 0, &[], "critical"), // 命中
            make_frame(2, 0.0, 0, &[], "warning"),
            make_frame(3, 0.0, 0, &[], "critical"), // 命中
        ];
        s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
        assert_eq!(s.matches, vec![1, 3]);
    }

    #[test]
    fn next_prev_navigation() {
        let mut s = ReplaySearch::new();
        s.matches = vec![5, 10, 15, 20];
        s.cursor = 0;

        assert_eq!(s.next_match(), Some(10));
        assert_eq!(s.next_match(), Some(15));
        assert_eq!(s.next_match(), Some(20));
        // clamp 在末尾
        assert_eq!(s.next_match(), Some(20));
        assert_eq!(s.prev_match(), Some(15));
        assert_eq!(s.prev_match(), Some(10));
        assert_eq!(s.prev_match(), Some(5));
        // clamp 在起点
        assert_eq!(s.prev_match(), Some(5));
    }

    #[test]
    fn next_match_on_empty_returns_none() {
        let mut s = ReplaySearch::new();
        assert!(s.next_match().is_none());
        assert!(s.prev_match().is_none());
    }

    #[test]
    fn reset_clears_all_state() {
        let mut s = ReplaySearch::new();
        for c in ":cpu > 5".chars() {
            s.push_char(c);
        }
        s.matches = vec![1, 2, 3];
        s.cursor = 1;
        assert!(s.is_active());

        s.reset();
        assert!(!s.is_active());
        assert!(s.input.is_empty());
        assert!(s.expr.is_none());
        assert!(s.matches.is_empty());
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn recompute_with_no_expr_clears_matches() {
        // 无 expr（input 为空 / 仅 `:`）→ matches 空
        let mut s = ReplaySearch::new();
        s.push_char(':');
        // 仅 `:` → parse_frame("") 应失败 / 输入为空 → expr 仍 None
        let frames = [make_frame(0, 100.0, 0, &[], "")];
        s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
        assert!(s.matches.is_empty());
    }

    #[test]
    fn current_match_returns_idx_at_cursor() {
        let mut s = ReplaySearch::new();
        s.matches = vec![10, 20, 30];
        s.cursor = 1;
        assert_eq!(s.current_match(), Some(20));
        s.cursor = 2;
        assert_eq!(s.current_match(), Some(30));
    }

    #[test]
    fn substring_search_escapes_regex_metachars() {
        // 用户输入 `chrome.exe` 不应被解释为 regex `chrome + 任意字符 + exe`。
        // build_frame_substring_expr 内部 escape，所以 `chrome.exe` 只匹配字面 `chrome.exe`。
        let mut s = ReplaySearch::new();
        for c in "chrome.exe".chars() {
            s.push_char(c);
        }
        let frames = [
            make_frame(0, 0.0, 0, &["chromexexe"], ""), // 不命中（regex 元字符被 escape）
            make_frame(1, 0.0, 0, &["chrome.exe"], ""), // 命中
            make_frame(2, 0.0, 0, &["chromeXexe"], ""), // 不命中
        ];
        s.recompute_matches(frames.len(), |i| Some(frames[i].clone()));
        assert_eq!(s.matches, vec![1]);
    }
}
