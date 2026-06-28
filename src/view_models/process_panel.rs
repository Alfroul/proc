use std::collections::HashMap;
use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent};

use crate::app_group::{self, AppGroup, AppGroupItem, VersionInfo, build_visual_items};
use crate::app_panel::{AppGroupSortField, AppMode, KeyResult, KillRequest, Panel, PanelContext};
use crate::classify;
use crate::collect::{ProcessInfo, ProcessViewMode, SortField};
use crate::tree::{self, TreeFilter, TreeNode};

const PAGE_SIZE: usize = 20;

pub struct ProcessPanel {
    // List view
    pub sort_field: SortField,
    pub cursor_index: usize,
    pub scroll_offset: usize,
    pub selected_pids: HashSet<u32>,
    pub search: crate::search::SearchState,

    // View mode
    pub process_view_mode: ProcessViewMode,

    // Tree view
    pub tree_nodes: Vec<TreeNode>,
    pub tree_filter: TreeFilter,
    pub tree_cursor: usize,
    pub tree_scroll: usize,
    pub tree_search: crate::search::SearchState,
    pub tree_selected_pids: HashSet<u32>,
    pub tree_sort_field: tree::TreeSortField,

    // AppGroup view
    pub app_groups: Vec<AppGroup>,
    pub app_group_cursor: usize,
    pub app_group_scroll: usize,
    pub app_group_expanded: Option<usize>,
    pub app_group_sort: AppGroupSortField,
    pub app_group_search: crate::search::SearchState,
    pub version_info_cache: HashMap<String, Option<VersionInfo>>,
}

impl ProcessPanel {
    #[must_use]
    pub fn new(_processes: &[ProcessInfo]) -> Self {
        let (_, _mem_total) = (0u64, 0u64); // caller provides this in App::new
        Self {
            sort_field: crate::ui_state::load_sort_field().unwrap_or_default(),
            cursor_index: 0,
            scroll_offset: 0,
            selected_pids: HashSet::new(),
            search: crate::search::SearchState::new(),
            process_view_mode: ProcessViewMode::List,
            tree_nodes: Vec::new(),
            tree_filter: TreeFilter::All,
            tree_cursor: 0,
            tree_scroll: 0,
            tree_search: crate::search::SearchState::new(),
            tree_selected_pids: HashSet::new(),
            tree_sort_field: tree::TreeSortField::Cpu,
            app_groups: Vec::new(),
            app_group_cursor: 0,
            app_group_scroll: 0,
            app_group_expanded: None,
            app_group_sort: AppGroupSortField::Cpu,
            app_group_search: crate::search::SearchState::new(),
            version_info_cache: HashMap::new(),
        }
    }

    pub fn init_tree(&mut self, processes: &[ProcessInfo], total_mem: u64) {
        self.tree_nodes = tree::build_process_tree(processes, total_mem);
    }

    #[must_use]
    pub fn filtered_count(&self, cached_sorted: &[(usize, classify::ProcessClass)]) -> usize {
        cached_sorted.len()
    }

    #[must_use]
    pub fn get_selected_pids(&self) -> Vec<u32> {
        self.selected_pids.iter().copied().collect()
    }

    // --- AppGroup helpers ---

    /// 计算 AppGroup 视图的扁平可视 item 列表。
    ///
    /// v0.8.0 阶段 3（TD-15）：加 `cached_processes` 参数支持 FilterExpr 模式。
    /// - Substring 模式：保留 v0.7 的「group.display_name / process.name / pid」模糊匹配。
    /// - FilterExpr 模式：
    ///   - **Header 项（聚合）**：用 group 的 `total_cpu` / `total_memory` 构造合成
    ///     ProcessInfo 再 apply。`cpu > 50` 表示「该 .exe 总 cpu > 50」。
    ///   - **Child 项（单进程）**：按 pid 查 cached_processes 拿原始 ProcessInfo 再 apply。
    ///   - Header 命中 → 整组保留（即使某些 child 不满足）；Header 不命中但 Child 命中
    ///     → 仅显示命中的 child（自动展开该组）。
    #[must_use]
    pub fn app_group_filtered_visual_items(
        &self,
        cached_processes: &[ProcessInfo],
    ) -> Vec<AppGroupItem> {
        match self.app_group_search.mode {
            crate::search::QueryMode::Substring => {
                if self.app_group_search.query().is_empty() {
                    return build_visual_items(&self.app_groups, self.app_group_expanded);
                }
                let query = self.app_group_search.query().to_lowercase();
                let filtered_groups: Vec<AppGroup> = self
                    .app_groups
                    .iter()
                    .filter(|g| {
                        g.display_name.to_lowercase().contains(&query)
                            || g.processes.iter().any(|p| {
                                p.name.to_lowercase().contains(&query)
                                    || p.pid.to_string().contains(self.app_group_search.query())
                            })
                    })
                    .cloned()
                    .collect();
                let mut expanded = None;
                if let Some(exp_idx) = self.app_group_expanded {
                    let mut count = 0;
                    for (i, g) in self.app_groups.iter().enumerate() {
                        if g.display_name.to_lowercase().contains(&query)
                            || g.processes.iter().any(|p| {
                                p.name.to_lowercase().contains(&query)
                                    || p.pid.to_string().contains(self.app_group_search.query())
                            })
                        {
                            if i == exp_idx {
                                expanded = Some(count);
                                break;
                            }
                            count += 1;
                        }
                    }
                }
                build_visual_items(&filtered_groups, expanded)
            }
            crate::search::QueryMode::FilterExpr => {
                let Some(expr) = self.app_group_search.filter_expr.as_ref() else {
                    return build_visual_items(&self.app_groups, self.app_group_expanded);
                };
                let pid_map: std::collections::HashMap<u32, &ProcessInfo> =
                    cached_processes.iter().map(|p| (p.pid, p)).collect();
                let mut items = Vec::new();
                for (gi, group) in self.app_groups.iter().enumerate() {
                    // Header：构造合成 ProcessInfo（聚合值）。
                    let synth = ProcessInfo {
                        name: std::sync::Arc::from(group.display_name.as_str()),
                        name_lower: std::sync::Arc::from(group.display_name.to_lowercase()),
                        cpu_usage: group.total_cpu,
                        memory: group.total_memory,
                        ..ProcessInfo::default()
                    };
                    let header_ctx = crate::filter::EvalCtx {
                        process: &synth,
                        security_score: None,
                    };
                    let header_match = expr.apply(&header_ctx);

                    if header_match {
                        // 整组保留：header + 当前 expanded 状态下的 children。
                        items.push(AppGroupItem::Header { group_idx: gi });
                        if self.app_group_expanded == Some(gi) {
                            for ci in 0..group.processes.len() {
                                items.push(AppGroupItem::Child {
                                    group_idx: gi,
                                    child_idx: ci,
                                });
                            }
                        }
                        continue;
                    }

                    // Header 不命中：逐 child 判断，命中的 child 保留并自动展开该组。
                    let matched_children: Vec<usize> = group
                        .processes
                        .iter()
                        .enumerate()
                        .filter_map(|(ci, p)| {
                            pid_map
                                .get(&p.pid)
                                .is_some_and(|proc| {
                                    let ctx = crate::filter::EvalCtx {
                                        process: proc,
                                        security_score: None,
                                    };
                                    expr.apply(&ctx)
                                })
                                .then_some(ci)
                        })
                        .collect();

                    if matched_children.is_empty() {
                        continue;
                    }

                    items.push(AppGroupItem::Header { group_idx: gi });
                    for ci in matched_children {
                        items.push(AppGroupItem::Child {
                            group_idx: gi,
                            child_idx: ci,
                        });
                    }
                }
                items
            }
        }
    }

    pub fn app_group_sort_groups(&mut self) {
        let sort = self.app_group_sort;
        self.app_groups.sort_by(|a, b| match sort {
            AppGroupSortField::Cpu => b
                .total_cpu
                .partial_cmp(&a.total_cpu)
                .unwrap_or(std::cmp::Ordering::Equal),
            AppGroupSortField::Memory => b.total_memory.cmp(&a.total_memory),
            AppGroupSortField::ProcessCount => b.processes.len().cmp(&a.processes.len()),
        });
    }

    // --- List view actions ---

    pub fn move_cursor(&mut self, delta: i32, cached_sorted: &[(usize, classify::ProcessClass)]) {
        let total = cached_sorted.len();
        if total == 0 {
            return;
        }
        let new = self.cursor_index as i32 + delta;
        self.cursor_index = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.clamp_scroll(PAGE_SIZE);
    }

    fn toggle_select(
        &mut self,
        cached_sorted: &[(usize, classify::ProcessClass)],
        cached_processes: &[ProcessInfo],
    ) {
        if let Some((idx, _)) = cached_sorted.get(self.cursor_index) {
            let pid = cached_processes[*idx].pid;
            if self.selected_pids.contains(&pid) {
                self.selected_pids.remove(&pid);
            } else {
                self.selected_pids.insert(pid);
            }
        }
    }

    fn select_all(
        &mut self,
        cached_sorted: &[(usize, classify::ProcessClass)],
        cached_processes: &[ProcessInfo],
    ) {
        for (idx, _) in cached_sorted {
            self.selected_pids.insert(cached_processes[*idx].pid);
        }
    }

    fn deselect_all(&mut self) {
        self.selected_pids.clear();
    }

    fn page_up(&mut self) {
        self.cursor_index = self.cursor_index.saturating_sub(PAGE_SIZE);
        self.clamp_scroll(PAGE_SIZE);
    }

    fn page_down(&mut self, cached_sorted: &[(usize, classify::ProcessClass)]) {
        let total = cached_sorted.len();
        self.cursor_index = (self.cursor_index + PAGE_SIZE).min(total.saturating_sub(1));
        self.clamp_scroll(PAGE_SIZE);
    }

    fn clamp_scroll(&mut self, page_size: usize) {
        if self.cursor_index < self.scroll_offset {
            self.scroll_offset = self.cursor_index;
        } else if self.cursor_index >= self.scroll_offset + page_size {
            self.scroll_offset = self.cursor_index - page_size + 1;
        }
    }

    fn enter_detail(
        &self,
        cached_sorted: &[(usize, classify::ProcessClass)],
        cached_processes: &[ProcessInfo],
    ) -> Option<ProcessInfo> {
        let (idx, _) = cached_sorted.get(self.cursor_index)?;
        Some(cached_processes[*idx].clone())
    }

    fn initiate_kill(
        &mut self,
        cached_sorted: &[(usize, classify::ProcessClass)],
        cached_processes: &[ProcessInfo],
        force: bool,
    ) -> Option<KillRequest> {
        let pids: Vec<u32> = if self.selected_pids.is_empty() {
            cached_sorted
                .get(self.cursor_index)
                .map(|(idx, _)| cached_processes[*idx].pid)
                .into_iter()
                .collect()
        } else {
            self.selected_pids.iter().copied().collect()
        };
        if pids.is_empty() {
            return None;
        }
        Some(KillRequest { pids, force })
    }

    // --- Tree view actions ---

    /// 计算当前 tree view 的可见节点列表。
    ///
    /// v0.8.0 阶段 3（TD-15）：加 `cached_processes` 参数支持 FilterExpr 模式。
    /// - Substring 模式：复用 v0.6 的 `name.to_lowercase().contains(query)` 逻辑。
    /// - FilterExpr 模式：用 pid 查 cached_processes 拿原始 ProcessInfo，再走
    ///   `FilterExpr::apply`。`security_score: None`（Tree 视图无 App::security_scores
    ///   访问路径；用户需安全分过滤时切 List 视图）。
    #[must_use]
    pub fn get_filtered_tree_visible(&self, cached_processes: &[ProcessInfo]) -> Vec<TreeNode> {
        let filtered = tree::filter_tree(&self.tree_nodes, self.tree_filter);
        let visible = tree::flatten_visible(&filtered);
        match self.tree_search.mode {
            crate::search::QueryMode::Substring => {
                if self.tree_search.query().is_empty() {
                    visible.into_iter().cloned().collect()
                } else {
                    let query_lower = self.tree_search.query().to_lowercase();
                    visible
                        .into_iter()
                        .filter(|n| {
                            n.name.to_lowercase().contains(&query_lower)
                                || n.pid.to_string().contains(self.tree_search.query())
                        })
                        .cloned()
                        .collect()
                }
            }
            crate::search::QueryMode::FilterExpr => {
                let Some(expr) = self.tree_search.filter_expr.as_ref() else {
                    // query 为空或 parse 失败且无先前 AST → 不过滤，与 substring 空 query 一致。
                    return visible.into_iter().cloned().collect();
                };
                // pid → &ProcessInfo 索引（一次构造 N 次查询）。
                let pid_map: std::collections::HashMap<u32, &ProcessInfo> =
                    cached_processes.iter().map(|p| (p.pid, p)).collect();
                visible
                    .into_iter()
                    .filter(|n| {
                        pid_map.get(&n.pid).is_some_and(|p| {
                            let ctx = crate::filter::EvalCtx {
                                process: p,
                                security_score: None,
                            };
                            expr.apply(&ctx)
                        })
                    })
                    .cloned()
                    .collect()
            }
        }
    }

    pub fn tree_move_cursor(&mut self, delta: i32, cached_processes: &[ProcessInfo]) {
        let visible = self.get_filtered_tree_visible(cached_processes);
        let total = visible.len();
        if total == 0 {
            return;
        }
        let new = self.tree_cursor as i32 + delta;
        self.tree_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.tree_clamp_scroll(PAGE_SIZE);
    }

    fn tree_toggle_expand(&mut self) {
        let pid = {
            let filtered = tree::filter_tree(&self.tree_nodes, self.tree_filter);
            let visible = tree::flatten_visible(&filtered);
            visible.get(self.tree_cursor).map(|n| n.pid)
        };
        if let Some(pid) = pid {
            tree::toggle_node_by_pid(&mut self.tree_nodes, pid);
        }
    }

    fn tree_toggle_select(&mut self, cached_processes: &[ProcessInfo]) {
        let visible = self.get_filtered_tree_visible(cached_processes);
        if let Some(node) = visible.get(self.tree_cursor) {
            let pid = node.pid;
            if self.tree_selected_pids.contains(&pid) {
                self.tree_selected_pids.remove(&pid);
            } else {
                self.tree_selected_pids.insert(pid);
            }
        }
    }

    fn tree_select_all(&mut self, cached_processes: &[ProcessInfo]) {
        let visible = self.get_filtered_tree_visible(cached_processes);
        for node in &visible {
            self.tree_selected_pids.insert(node.pid);
        }
    }

    fn tree_deselect_all(&mut self) {
        self.tree_selected_pids.clear();
    }

    fn tree_initiate_kill(
        &mut self,
        force: bool,
        cached_processes: &[ProcessInfo],
    ) -> Option<KillRequest> {
        let pids: Vec<u32> = if self.tree_selected_pids.is_empty() {
            self.get_filtered_tree_visible(cached_processes)
                .get(self.tree_cursor)
                .map(|n| n.pid)
                .into_iter()
                .collect()
        } else {
            self.tree_selected_pids.iter().copied().collect()
        };
        if pids.is_empty() {
            return None;
        }
        Some(KillRequest { pids, force })
    }

    fn tree_select_orphans(&mut self, cached_processes: &[ProcessInfo]) -> Option<String> {
        let visible = self.get_filtered_tree_visible(cached_processes);
        let orphan_pids: Vec<u32> = visible
            .iter()
            .filter(|n| n.is_orphan)
            .map(|n| n.pid)
            .collect();
        if orphan_pids.is_empty() {
            return Some("无孤儿进程".to_string());
        }
        self.tree_selected_pids = orphan_pids.into_iter().collect();
        let total_mem: u64 = visible
            .iter()
            .filter(|n| n.is_orphan && self.tree_selected_pids.contains(&n.pid))
            .map(|n| n.memory)
            .sum();
        let safe_count = visible
            .iter()
            .filter(|n| {
                n.is_orphan && n.children.is_empty() && self.tree_selected_pids.contains(&n.pid)
            })
            .count();
        Some(format!(
            "{}个孤儿 | 可直接终止:{} | 共{} | Space取消 | k终止",
            self.tree_selected_pids.len(),
            safe_count,
            crate::format::format_bytes(total_mem)
        ))
    }

    fn tree_select_stale(&mut self, cached_processes: &[ProcessInfo]) -> Option<String> {
        let visible = self.get_filtered_tree_visible(cached_processes);
        let stale_pids: Vec<u32> = visible
            .iter()
            .filter(|n| n.is_zombie || n.is_stale)
            .map(|n| n.pid)
            .collect();
        if stale_pids.is_empty() {
            return Some("无僵尸/残存进程".to_string());
        }
        self.tree_selected_pids = stale_pids.into_iter().collect();
        let total_mem: u64 = visible
            .iter()
            .filter(|n| (n.is_zombie || n.is_stale) && self.tree_selected_pids.contains(&n.pid))
            .map(|n| n.memory)
            .sum();
        let safe_count = visible
            .iter()
            .filter(|n| {
                (n.is_zombie || n.is_stale)
                    && n.children.is_empty()
                    && self.tree_selected_pids.contains(&n.pid)
            })
            .count();
        Some(format!(
            "{}个残存 | 可直接终止:{} | 共{} | Space取消 | k终止",
            self.tree_selected_pids.len(),
            safe_count,
            crate::format::format_bytes(total_mem)
        ))
    }

    fn tree_cycle_filter(&mut self) {
        self.tree_filter = match self.tree_filter {
            TreeFilter::All => TreeFilter::MyProcesses,
            TreeFilter::MyProcesses => TreeFilter::SystemProcesses,
            TreeFilter::SystemProcesses => TreeFilter::All,
        };
        self.tree_cursor = 0;
        self.tree_scroll = 0;
    }

    fn tree_clamp_scroll(&mut self, page_size: usize) {
        if self.tree_cursor < self.tree_scroll {
            self.tree_scroll = self.tree_cursor;
        } else if self.tree_cursor >= self.tree_scroll + page_size {
            self.tree_scroll = self.tree_cursor - page_size + 1;
        }
    }

    // --- AppGroup view actions ---

    pub fn app_group_move_cursor(&mut self, delta: i32, cached_processes: &[ProcessInfo]) {
        let items = self.app_group_filtered_visual_items(cached_processes);
        let total = items.len();
        if total == 0 {
            return;
        }
        let new = self.app_group_cursor as i32 + delta;
        self.app_group_cursor = if new < 0 {
            total - 1
        } else if new as usize >= total {
            0
        } else {
            new as usize
        };
        self.app_group_clamp_scroll(PAGE_SIZE);
    }

    fn app_group_toggle_expand(&mut self, cached_processes: &[ProcessInfo]) {
        let items = self.app_group_filtered_visual_items(cached_processes);
        if let Some(item) = items.get(self.app_group_cursor) {
            match *item {
                AppGroupItem::Header { group_idx } | AppGroupItem::Child { group_idx, .. } => {
                    self.app_group_expanded = if self.app_group_expanded == Some(group_idx) {
                        None
                    } else {
                        Some(group_idx)
                    };
                }
            }
        }
    }

    fn app_group_toggle_select(&mut self, cached_processes: &[ProcessInfo]) {
        let items = self.app_group_filtered_visual_items(cached_processes);
        if let Some(item) = items.get(self.app_group_cursor) {
            let pid = match *item {
                AppGroupItem::Header { .. } => return,
                AppGroupItem::Child {
                    group_idx,
                    child_idx,
                } => self
                    .app_groups
                    .get(group_idx)
                    .and_then(|g| g.processes.get(child_idx))
                    .map(|p| p.pid),
            };
            if let Some(pid) = pid {
                if self.selected_pids.contains(&pid) {
                    self.selected_pids.remove(&pid);
                } else {
                    self.selected_pids.insert(pid);
                }
            }
        }
    }

    fn app_group_initiate_kill(
        &mut self,
        force: bool,
        cached_processes: &[ProcessInfo],
    ) -> Option<KillRequest> {
        let items = self.app_group_filtered_visual_items(cached_processes);
        let mut pids: Vec<u32> = Vec::new();
        if !self.selected_pids.is_empty() {
            for group in &self.app_groups {
                for proc in &group.processes {
                    if self.selected_pids.contains(&proc.pid) {
                        pids.push(proc.pid);
                    }
                }
            }
        }
        if pids.is_empty()
            && let Some(item) = items.get(self.app_group_cursor)
        {
            match *item {
                AppGroupItem::Header { group_idx } => {
                    if let Some(group) = self.app_groups.get(group_idx) {
                        pids = group.processes.iter().map(|p| p.pid).collect();
                    }
                }
                AppGroupItem::Child {
                    group_idx,
                    child_idx,
                } => {
                    if let Some(group) = self.app_groups.get(group_idx)
                        && let Some(proc) = group.processes.get(child_idx)
                    {
                        pids.push(proc.pid);
                    }
                }
            }
        }
        if pids.is_empty() {
            return None;
        }
        Some(KillRequest { pids, force })
    }

    fn app_group_clamp_scroll(&mut self, page_size: usize) {
        if self.app_group_cursor < self.app_group_scroll {
            self.app_group_scroll = self.app_group_cursor;
        } else if self.app_group_cursor >= self.app_group_scroll + page_size {
            self.app_group_scroll = self.app_group_cursor - page_size + 1;
        }
    }

    fn toggle_view_mode(&mut self, cached_processes: &[ProcessInfo]) -> String {
        self.process_view_mode = self.process_view_mode.toggle();
        self.cursor_index = 0;
        self.scroll_offset = 0;
        self.tree_cursor = 0;
        self.tree_scroll = 0;
        self.app_group_cursor = 0;
        self.app_group_scroll = 0;
        self.app_group_expanded = None;
        let label = self.process_view_mode.label().to_string();
        if self.process_view_mode == ProcessViewMode::AppGroup {
            self.app_groups =
                app_group::compute_groups(cached_processes, &mut self.version_info_cache);
        }
        label
    }

    // --- Key handlers by view mode ---

    fn handle_list_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
            KeyCode::Up => self.move_cursor(-1, ctx.cached_sorted),
            KeyCode::Down => self.move_cursor(1, ctx.cached_sorted),
            KeyCode::Left => {
                self.sort_field = self.sort_field.prev();
                crate::ui_state::save_sort_field(self.sort_field);
                self.cursor_index = 0;
                self.scroll_offset = 0;
                *ctx.data_dirty = true;
            }
            KeyCode::Right => {
                self.sort_field = self.sort_field.next();
                crate::ui_state::save_sort_field(self.sort_field);
                self.cursor_index = 0;
                self.scroll_offset = 0;
                *ctx.data_dirty = true;
            }
            KeyCode::Char(' ') => self.toggle_select(ctx.cached_sorted, ctx.cached_processes),
            KeyCode::Char('a') => self.select_all(ctx.cached_sorted, ctx.cached_processes),
            KeyCode::Char('A') => self.deselect_all(),
            KeyCode::Char('/') => {
                self.search.active = true;
            }
            // v0.7 阶段 4：':' 激活 FilterExpr 模式（ADR-0011）。
            // v0.8 阶段 3：Tree / AppGroup 视图同款接入（TD-15），三视图均支持。
            KeyCode::Char(':') => {
                self.search.activate_filter_expr();
            }
            KeyCode::Enter => {
                if let Some(proc) = self.enter_detail(ctx.cached_sorted, ctx.cached_processes) {
                    *ctx.detail_process = Some(proc);
                    return KeyResult::SwitchMode(AppMode::ProcessDetail);
                }
            }
            KeyCode::Char('k') => {
                if let Some(req) =
                    self.initiate_kill(ctx.cached_sorted, ctx.cached_processes, false)
                {
                    *ctx.pending_kill = Some(req);
                    return KeyResult::Consumed; // App will set kill_confirm
                }
            }
            KeyCode::Char('K') => {
                if let Some(req) = self.initiate_kill(ctx.cached_sorted, ctx.cached_processes, true)
                {
                    *ctx.pending_kill = Some(req);
                    return KeyResult::Consumed;
                }
            }
            KeyCode::Char('S') => {
                self.sort_field = SortField::Security;
                crate::ui_state::save_sort_field(self.sort_field);
                self.cursor_index = 0;
                self.scroll_offset = 0;
                *ctx.data_dirty = true;
            }
            KeyCode::Char('v') => {
                let label = self.toggle_view_mode(ctx.cached_processes);
                *ctx.status_message = Some(format!("视图: {}", label));
            }
            // 阶段 4 A4：进程列表 `+`/`-` 调整选中进程的优先级。
            // 不依赖 App::bump_selected_priority，直接调 process_control 把
            // 错误/成功写到 status_message，保持 PanelContext 不需要新增字段。
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Some(pid) = self.focused_pid(ctx.cached_sorted, ctx.cached_processes) {
                    bump_priority_into(pid, true, ctx.status_message);
                }
            }
            KeyCode::Char('-') => {
                if let Some(pid) = self.focused_pid(ctx.cached_sorted, ctx.cached_processes) {
                    bump_priority_into(pid, false, ctx.status_message);
                }
            }
            KeyCode::PageUp => self.page_up(),
            KeyCode::PageDown => self.page_down(ctx.cached_sorted),
            KeyCode::Esc => {
                self.search.clear();
                *ctx.status_message = None;
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    fn handle_tree_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
            KeyCode::Up => self.tree_move_cursor(-1, ctx.cached_processes),
            KeyCode::Down => self.tree_move_cursor(1, ctx.cached_processes),
            KeyCode::Enter => self.tree_toggle_expand(),
            KeyCode::Left => {
                self.tree_sort_field = self.tree_sort_field.prev();
                tree::sort_siblings(&mut self.tree_nodes, self.tree_sort_field);
                *ctx.status_message = Some(format!("排序: {}", self.tree_sort_field.label()));
            }
            KeyCode::Right => {
                self.tree_sort_field = self.tree_sort_field.next();
                tree::sort_siblings(&mut self.tree_nodes, self.tree_sort_field);
                *ctx.status_message = Some(format!("排序: {}", self.tree_sort_field.label()));
            }
            KeyCode::Char('/') => {
                self.tree_search.active = true;
            }
            // v0.8 阶段 3（TD-15）：':' 激活 FilterExpr 模式。
            KeyCode::Char(':') => {
                self.tree_search.activate_filter_expr();
            }
            KeyCode::Char(' ') => self.tree_toggle_select(ctx.cached_processes),
            KeyCode::Char('a') => self.tree_select_all(ctx.cached_processes),
            KeyCode::Char('A') => self.tree_deselect_all(),
            KeyCode::Char('k') => {
                if let Some(req) = self.tree_initiate_kill(false, ctx.cached_processes) {
                    *ctx.pending_kill = Some(req);
                    return KeyResult::Consumed;
                }
            }
            KeyCode::Char('K') => {
                if let Some(req) = self.tree_initiate_kill(true, ctx.cached_processes) {
                    *ctx.pending_kill = Some(req);
                    return KeyResult::Consumed;
                }
            }
            KeyCode::Char('o') => {
                if let Some(msg) = self.tree_select_orphans(ctx.cached_processes) {
                    *ctx.status_message = Some(msg);
                }
            }
            KeyCode::Char('z') => {
                if let Some(msg) = self.tree_select_stale(ctx.cached_processes) {
                    *ctx.status_message = Some(msg);
                }
            }
            KeyCode::Char('f') => self.tree_cycle_filter(),
            KeyCode::Esc => {
                self.tree_search.clear();
                *ctx.status_message = None;
            }
            KeyCode::Char('v') => {
                let label = self.toggle_view_mode(ctx.cached_processes);
                *ctx.status_message = Some(format!("视图: {}", label));
            }
            KeyCode::PageUp => {
                self.tree_cursor = self.tree_cursor.saturating_sub(PAGE_SIZE);
                self.tree_clamp_scroll(PAGE_SIZE);
            }
            KeyCode::PageDown => {
                let visible = self.get_filtered_tree_visible(ctx.cached_processes);
                let total = visible.len();
                self.tree_cursor = (self.tree_cursor + PAGE_SIZE).min(total.saturating_sub(1));
                self.tree_clamp_scroll(PAGE_SIZE);
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    fn handle_app_group_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        match key.code {
            KeyCode::Char('q') => return KeyResult::Quit,
            KeyCode::Up => self.app_group_move_cursor(-1, ctx.cached_processes),
            KeyCode::Down => self.app_group_move_cursor(1, ctx.cached_processes),
            KeyCode::Enter => self.app_group_toggle_expand(ctx.cached_processes),
            KeyCode::Char(' ') => self.app_group_toggle_select(ctx.cached_processes),
            KeyCode::Char('/') => {
                self.app_group_search.active = true;
            }
            // v0.8 阶段 3（TD-15）：':' 激活 FilterExpr 模式。
            KeyCode::Char(':') => {
                self.app_group_search.activate_filter_expr();
            }
            KeyCode::Char('k') => {
                if let Some(req) = self.app_group_initiate_kill(false, ctx.cached_processes) {
                    *ctx.pending_kill = Some(req);
                    return KeyResult::Consumed;
                }
            }
            KeyCode::Char('K') => {
                if let Some(req) = self.app_group_initiate_kill(true, ctx.cached_processes) {
                    *ctx.pending_kill = Some(req);
                    return KeyResult::Consumed;
                }
            }
            KeyCode::Char('S') => {
                self.app_group_sort = self.app_group_sort.next();
                self.app_group_sort_groups();
                *ctx.status_message = Some(format!("排序: {}", self.app_group_sort.label()));
            }
            KeyCode::Char('v') => {
                let label = self.toggle_view_mode(ctx.cached_processes);
                *ctx.status_message = Some(format!("视图: {}", label));
            }
            KeyCode::PageUp => {
                self.app_group_cursor = self.app_group_cursor.saturating_sub(PAGE_SIZE);
                self.app_group_clamp_scroll(PAGE_SIZE);
            }
            KeyCode::PageDown => {
                let items = self.app_group_filtered_visual_items(ctx.cached_processes);
                let total = items.len();
                self.app_group_cursor =
                    (self.app_group_cursor + PAGE_SIZE).min(total.saturating_sub(1));
                self.app_group_clamp_scroll(PAGE_SIZE);
            }
            KeyCode::Esc => {
                self.app_group_search.clear();
                *ctx.status_message = None;
            }
            _ => return KeyResult::Ignored,
        }
        KeyResult::Consumed
    }

    pub fn refresh_tree(&mut self, processes: &[ProcessInfo], total_mem: u64) {
        let expanded_pids = tree::collect_expanded_pids(&self.tree_nodes);
        self.tree_nodes = tree::build_process_tree(processes, total_mem);
        tree::restore_expanded_pids(&mut self.tree_nodes, &expanded_pids);
        tree::sort_siblings(&mut self.tree_nodes, self.tree_sort_field);
    }

    pub fn rebuild_app_groups(&mut self, processes: &[ProcessInfo]) {
        let prev_expanded = self.app_group_expanded;
        self.app_groups = app_group::compute_groups(processes, &mut self.version_info_cache);
        // Evict stale cache entries
        if self.version_info_cache.len() > 200 {
            let active_exes: HashSet<String> = processes
                .iter()
                .filter_map(|p| p.exe.as_ref().map(|e| (*e).to_string()))
                .collect();
            self.version_info_cache
                .retain(|k, _| active_exes.contains(k));
        }
        self.app_group_sort_groups();
        if prev_expanded.is_some() && prev_expanded.unwrap() >= self.app_groups.len() {
            self.app_group_expanded = None;
        } else {
            self.app_group_expanded = prev_expanded;
        }
    }

    /// 列表视图下当前焦点 PID：优先用多选集合里的「最后选中」，否则用 cursor。
    /// 用于 `+`/`-` 调优先级等单进程操作。
    fn focused_pid(
        &self,
        cached_sorted: &[(usize, classify::ProcessClass)],
        cached_processes: &[ProcessInfo],
    ) -> Option<u32> {
        if let Some(&last) = self.selected_pids.iter().last() {
            return Some(last);
        }
        let (idx, _) = cached_sorted.get(self.cursor_index)?;
        cached_processes.get(*idx).map(|p| p.pid)
    }
}

/// A4：把 get/set_priority 的结果直接写进 `status_message`。失败原因可能是
/// 权限不足（非管理员）、进程已退出、或平台不支持（macOS）。把状态往上调比
/// ProcessPanel 自己拼字符串省事。
fn bump_priority_into(pid: u32, up: bool, status: &mut Option<String>) {
    use crate::process_control::{get_priority, set_priority};
    let current = match get_priority(pid) {
        Ok(c) => c,
        Err(e) => {
            *status = Some(format!("读取优先级失败: {}", e));
            return;
        }
    };
    let next = if up {
        current.bump_up()
    } else {
        current.bump_down()
    };
    if next == current {
        *status = Some(format!(
            "PID {} 已到达 {} 端",
            pid,
            if up { "Realtime" } else { "Idle" }
        ));
        return;
    }
    match set_priority(pid, next) {
        Ok(()) => {
            let verb = if up { "调高至" } else { "调低至" };
            *status = Some(format!("PID {} 优先级 {} {}", pid, verb, next.label()));
        }
        Err(e) => {
            *status = Some(format!(
                "PID {} 设置优先级失败 ({} → {}): {}",
                pid,
                current.label(),
                next.label(),
                e
            ));
        }
    }
}

impl Panel for ProcessPanel {
    fn handle_key(&mut self, key: KeyEvent, ctx: &mut PanelContext) -> KeyResult {
        // Handle search for current view mode
        match self.process_view_mode {
            ProcessViewMode::List => {
                if self.search.is_active() {
                    if self.search.handle_input(key) {
                        self.cursor_index = 0;
                        self.scroll_offset = 0;
                        // 每次按键都重建 cached_sorted，让用户立即看到过滤结果。
                        // （之前只在 query.len() <= 1 时设 data_dirty，导致按第 2+ 字符
                        // 时 UI 不更新，体感像「搜索响应慢」—— 要等 heavy tick ~1.5s
                        // 才重建。rebuild_sorted_cache 本身 O(N log N)，N~数百进程
                        // 时 < 1ms，每帧重算可接受。）
                        *ctx.data_dirty = true;
                    }
                    return KeyResult::Consumed;
                }
            }
            ProcessViewMode::Tree => {
                if self.tree_search.is_active() {
                    if self.tree_search.handle_input(key) {
                        self.tree_cursor = 0;
                        self.tree_scroll = 0;
                    }
                    return KeyResult::Consumed;
                }
            }
            ProcessViewMode::AppGroup => {
                if self.app_group_search.is_active() {
                    if self.app_group_search.handle_input(key) {
                        self.app_group_cursor = 0;
                        self.app_group_scroll = 0;
                    }
                    return KeyResult::Consumed;
                }
            }
        }

        match self.process_view_mode {
            ProcessViewMode::Tree => self.handle_tree_key(key, ctx),
            ProcessViewMode::AppGroup => self.handle_app_group_key(key, ctx),
            ProcessViewMode::List => self.handle_list_key(key, ctx),
        }
    }

    fn tick(&mut self, ctx: &mut PanelContext) -> bool {
        let mut needs_draw = false;
        if self.process_view_mode == ProcessViewMode::Tree {
            self.refresh_tree(ctx.cached_processes, ctx.snapshot.memory_usage().1);
            needs_draw = true;
        } else if self.process_view_mode == ProcessViewMode::AppGroup {
            self.rebuild_app_groups(ctx.cached_processes);
            needs_draw = true;
        }

        // Clamp cursors so 进程退出 / 搜索收窄后光标不至于指向越界位置。
        // tree_move_cursor / app_group_move_cursor 走 wraparound，不 clamp 时
        // 第一次按键会产生奇怪跳变；list 模式靠 cursor 本身 saturating。
        match self.process_view_mode {
            ProcessViewMode::List => {
                let total = ctx.cached_sorted.len();
                if total == 0 {
                    self.cursor_index = 0;
                } else if self.cursor_index >= total {
                    self.cursor_index = total - 1;
                }
            }
            ProcessViewMode::Tree => {
                let total = self.get_filtered_tree_visible(ctx.cached_processes).len();
                if total == 0 {
                    self.tree_cursor = 0;
                } else if self.tree_cursor >= total {
                    self.tree_cursor = total - 1;
                }
            }
            ProcessViewMode::AppGroup => {
                let total = self.app_groups.len();
                if total == 0 {
                    self.app_group_cursor = 0;
                } else if self.app_group_cursor >= total {
                    self.app_group_cursor = total - 1;
                }
            }
        }

        needs_draw
    }

    fn cursor(&self) -> usize {
        match self.process_view_mode {
            ProcessViewMode::List => self.cursor_index,
            ProcessViewMode::Tree => self.tree_cursor,
            ProcessViewMode::AppGroup => self.app_group_cursor,
        }
    }

    fn scroll(&self) -> usize {
        match self.process_view_mode {
            ProcessViewMode::List => self.scroll_offset,
            ProcessViewMode::Tree => self.tree_scroll,
            ProcessViewMode::AppGroup => self.app_group_scroll,
        }
    }
}
