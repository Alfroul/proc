use std::collections::HashMap;

use crate::classify;
use crate::collect::ProcessInfo;

/// 进程树节点
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KillSafety {
    Safe,
    Caution,
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
    pub depth: usize,
    pub children: Vec<TreeNode>,
    pub expanded: bool,
    pub class: classify::ProcessClass,
    pub is_orphan: bool,
    pub is_zombie: bool,
    pub is_stale: bool,
    pub kill_safety: Option<KillSafety>,
}

/// 树过滤模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TreeFilter {
    #[default]
    All,
    MyProcesses,
    SystemProcesses,
}

/// 构建进程树
pub fn build_process_tree(processes: &[ProcessInfo]) -> Vec<TreeNode> {
    // 构建 parent_pid → children PIDs 映射
    let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut proc_map: HashMap<u32, &ProcessInfo> = HashMap::new();

    for proc in processes {
        proc_map.insert(proc.pid, proc);
        if let Some(ppid) = proc.parent_pid {
            children_map.entry(ppid).or_default().push(proc.pid);
        }
    }

    // 识别根节点：parent_pid 不在进程列表中，或 parent_pid == 0
    let roots: Vec<u32> = processes
        .iter()
        .filter(|p| {
            p.parent_pid.is_none()
                || p.parent_pid == Some(0)
                || p.parent_pid.is_none_or(|ppid| !proc_map.contains_key(&ppid))
        })
        .map(|p| p.pid)
        .collect();

    // DFS 构建树（防循环）
    let mut visited = std::collections::HashSet::new();
    let mut result = Vec::new();
    for root_pid in roots {
        if let Some(node) = build_node(root_pid, 0, &children_map, &proc_map, &mut visited) {
            result.push(node);
        }
    }

    // 按名称排序根节点
    result.sort_by_key(|a| a.name.to_lowercase());
    result
}

fn build_node(
    pid: u32,
    depth: usize,
    children_map: &HashMap<u32, Vec<u32>>,
    proc_map: &HashMap<u32, &ProcessInfo>,
    visited: &mut std::collections::HashSet<u32>,
) -> Option<TreeNode> {
    if visited.contains(&pid) {
        return None;
    }
    visited.insert(pid);

    let proc = proc_map.get(&pid)?;

    let class = classify::classify_process(proc);
    let is_user = matches!(class, classify::ProcessClass::UserApp);

    let name_lower = proc.name.to_lowercase();
    let is_expected_orphan = classify::EXPECTED_ORPHAN_NAMES
        .iter()
        .any(|n| n.to_lowercase() == name_lower);

    let is_orphan = is_user
        && !is_expected_orphan
        && pid > 4
        && proc.parent_pid.is_some_and(|ppid| {
            ppid > 0 && ppid != 4 && !proc_map.contains_key(&ppid)
        });

    let is_zombie = proc.status == "Zombie";

    let is_stale = !is_zombie
        && is_user
        && proc.memory == 0
        && proc.cpu_usage == 0.0
        && proc.run_time > 3600;

    let mut children = Vec::new();
    if let Some(child_pids) = children_map.get(&pid) {
        for &child_pid in child_pids {
            if let Some(child) = build_node(child_pid, depth + 1, children_map, proc_map, visited) {
                children.push(child);
            }
        }
        children.sort_by_key(|a| a.name.to_lowercase());
    }

    let kill_safety = if is_orphan || is_zombie || is_stale {
        Some(if children.is_empty() {
            KillSafety::Safe
        } else {
            KillSafety::Caution
        })
    } else {
        None
    };

    Some(TreeNode {
        pid: proc.pid,
        name: proc.name.clone(),
        cpu: proc.cpu_usage,
        memory: proc.memory,
        depth,
        children,
        expanded: depth < 1,
        class,
        is_orphan,
        is_zombie,
        is_stale,
        kill_safety,
    })
}

pub fn count_anomalies(tree: &[TreeNode]) -> (usize, usize) {
    let mut orphans = 0;
    let mut zombies = 0;
    count_anomalies_recursive(tree, &mut orphans, &mut zombies);
    (orphans, zombies)
}

fn count_anomalies_recursive(tree: &[TreeNode], orphans: &mut usize, zombies: &mut usize) {
    for node in tree {
        if node.is_orphan {
            *orphans += 1;
        }
        if node.is_zombie || node.is_stale {
            *zombies += 1;
        }
        count_anomalies_recursive(&node.children, orphans, zombies);
    }
}

/// 按分类过滤进程树
pub fn filter_tree(tree: &[TreeNode], filter: TreeFilter) -> Vec<TreeNode> {
    match filter {
        TreeFilter::All => tree.to_vec(),
        TreeFilter::MyProcesses => filter_nodes(tree, |n| {
            matches!(n.class, classify::ProcessClass::UserApp)
        }),
        TreeFilter::SystemProcesses => filter_nodes(tree, |n| {
            matches!(
                n.class,
                classify::ProcessClass::SystemProcess
                    | classify::ProcessClass::WindowsService
                    | classify::ProcessClass::Kernel
            )
        }),
    }
}

fn filter_nodes<F>(nodes: &[TreeNode], pred: F) -> Vec<TreeNode>
where
    F: Fn(&TreeNode) -> bool + Copy,
{
    nodes
        .iter()
        .filter_map(|node| {
            let filtered_children = filter_nodes(&node.children, pred);
            if pred(node) || !filtered_children.is_empty() {
                let mut clone = node.clone();
                clone.children = filtered_children;
                Some(clone)
            } else {
                None
            }
        })
        .collect()
}

/// 将树扁平化为可见行（根据展开状态）
pub fn flatten_visible(tree: &[TreeNode]) -> Vec<&TreeNode> {
    let mut result = Vec::new();
    for node in tree {
        flatten_node(node, &mut result);
    }
    result
}

fn flatten_node<'a>(node: &'a TreeNode, result: &mut Vec<&'a TreeNode>) {
    result.push(node);
    if node.expanded {
        for child in &node.children {
            flatten_node(child, result);
        }
    }
}

/// 搜索树中匹配的节点（模糊匹配名称或 PID）
pub fn search_tree<'a>(tree: &'a [TreeNode], query: &str) -> Vec<&'a TreeNode> {
    let query_lower = query.to_lowercase();
    let mut result = Vec::new();
    for node in tree {
        search_node(node, &query_lower, &mut result);
    }
    result
}

fn search_node<'a>(
    node: &'a TreeNode,
    query: &str,
    result: &mut Vec<&'a TreeNode>,
) {
    if node.name.to_lowercase().contains(query)
        || node.pid.to_string().contains(query)
    {
        result.push(node);
    }
    for child in &node.children {
        search_node(child, query, result);
    }
}

/// 文本格式化输出树（CLI 模式）
pub fn format_tree_text(tree: &[TreeNode]) -> String {
    let mut lines = Vec::new();
    for (i, node) in tree.iter().enumerate() {
        let is_last = i == tree.len() - 1;
        format_node_text(node, "", is_last, &mut lines);
    }
    lines.join("\n")
}

fn format_node_text(
    node: &TreeNode,
    prefix: &str,
    is_last: bool,
    lines: &mut Vec<String>,
) {
    let connector = if is_last { "└─ " } else { "├─ " };
    let line = format!(
        "{}{}[{}] {} (PID {}, CPU {:.1}%, MEM {})",
        prefix,
        connector,
        node.class.label(),
        node.name,
        node.pid,
        node.cpu,
        format_bytes(node.memory),
    );
    lines.push(line);

    let child_prefix = format!("{}{}", prefix, if is_last { "   " } else { "│  " });
    for (i, child) in node.children.iter().enumerate() {
        let child_is_last = i == node.children.len() - 1;
        format_node_text(child, &child_prefix, child_is_last, lines);
    }
}

use crate::format::format_bytes;

/// 格式化进程信息为可读文本（剪贴板复制用）
pub fn format_process_info(proc: &ProcessInfo) -> String {
    format!(
        "PID: {}\n名称: {}\nCPU: {:.1}%\n内存: {} bytes\n虚拟内存: {} bytes\n状态: {}\n命令: {}\n工作目录: {}\n可执行: {}",
        proc.pid,
        proc.name,
        proc.cpu_usage,
        proc.memory,
        proc.virtual_memory,
        proc.status,
        proc.cmd.join(" "),
        proc.cwd.as_deref().unwrap_or("-"),
        proc.exe.as_deref().unwrap_or("-"),
    )
}

/// 切换指定 PID 节点的展开/折叠状态
pub fn toggle_node_by_pid(nodes: &mut [TreeNode], target_pid: u32) -> bool {
    for node in nodes.iter_mut() {
        if node.pid == target_pid {
            node.expanded = !node.expanded;
            return true;
        }
        if toggle_node_by_pid(&mut node.children, target_pid) {
            return true;
        }
    }
    false
}

/// 收集当前已展开的节点 PID 集合
pub fn collect_expanded_pids(nodes: &[TreeNode]) -> std::collections::HashSet<u32> {
    let mut pids = std::collections::HashSet::new();
    for node in nodes {
        if node.expanded {
            pids.insert(node.pid);
        }
        pids.extend(collect_expanded_pids(&node.children));
    }
    pids
}

/// 根据已展开 PID 集合恢复展开状态
pub fn restore_expanded_pids(nodes: &mut [TreeNode], expanded: &std::collections::HashSet<u32>) {
    for node in nodes.iter_mut() {
        node.expanded = expanded.contains(&node.pid);
        restore_expanded_pids(&mut node.children, expanded);
    }
}
