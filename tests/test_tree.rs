use proc::collect::ProcessInfo;
use proc::tree::{self, TreeFilter};

fn make_process(pid: u32, name: &str, parent_pid: Option<u32>) -> ProcessInfo {
    ProcessInfo {
        pid,
        name: name.to_string(),
        cpu_usage: 0.0,
        memory: 0,
        virtual_memory: 0,
        disk_usage: (0, 0),
        disk_read_speed: 0,
        disk_write_speed: 0,
        status: "Run".to_string(),
        exe: None,
        cmd: vec![],
        cwd: None,
        parent_pid,
        session_id: None,
        user_id: None,
        start_time: 0,
        run_time: 0,
    }
}

#[test]
fn test_tree_build_simple_parent_child() {
    let processes = vec![
        make_process(1, "init", None),
        make_process(10, "child_a", Some(1)),
        make_process(20, "child_b", Some(1)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    assert_eq!(tree.len(), 1, "should have one root");
    assert_eq!(tree[0].pid, 1);
    assert_eq!(tree[0].children.len(), 2);
}

#[test]
fn test_tree_root_identification() {
    let processes = vec![
        make_process(1, "root", Some(9999)), // parent 9999 doesn't exist → root
        make_process(2, "child", Some(1)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].pid, 1);
}

#[test]
fn test_tree_root_with_parent_zero() {
    let processes = vec![
        make_process(0, "idle", Some(0)),
        make_process(4, "system", Some(0)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    assert!(!tree.is_empty());
}

#[test]
fn test_tree_no_parent() {
    let processes = vec![make_process(100, "orphan", None)];

    let tree = tree::build_process_tree(&processes, 0);
    assert_eq!(tree.len(), 1);
    assert_eq!(tree[0].pid, 100);
}

#[test]
fn test_tree_depth() {
    let processes = vec![
        make_process(1, "a", None),
        make_process(2, "b", Some(1)),
        make_process(3, "c", Some(2)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    assert_eq!(tree[0].depth, 0);
    assert_eq!(tree[0].children[0].depth, 1);
    assert_eq!(tree[0].children[0].children[0].depth, 2);
}

#[test]
fn test_tree_filter_all() {
    let processes = vec![
        make_process(1, "init", None),
        make_process(4, "System", None),
        make_process(100, "myapp", Some(1)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    let filtered = tree::filter_tree(&tree, TreeFilter::All);
    assert!(filtered.len() >= 2);
}

#[test]
fn test_tree_filter_my_processes() {
    let processes = vec![
        make_process(1, "init", None),
        make_process(100, "myapp", Some(1)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    let filtered = tree::filter_tree(&tree, TreeFilter::MyProcesses);
    let all_pids: Vec<u32> = tree::flatten_visible(&filtered)
        .iter()
        .map(|n| n.pid)
        .collect();
    assert!(all_pids.contains(&100));
}

#[test]
fn test_tree_filter_system_processes() {
    let processes = vec![
        make_process(4, "System", None),
        make_process(100, "myapp", None),
        make_process(10, "child_sys", Some(4)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    let filtered = tree::filter_tree(&tree, TreeFilter::SystemProcesses);
    let all_pids: Vec<u32> = tree::flatten_visible(&filtered)
        .iter()
        .map(|n| n.pid)
        .collect();
    assert!(all_pids.contains(&4));
    assert!(!all_pids.contains(&100));
}

#[test]
fn test_tree_flatten_visible() {
    let processes = vec![
        make_process(1, "root", None),
        make_process(10, "child1", Some(1)),
        make_process(20, "child2", Some(1)),
        make_process(100, "grandchild", Some(10)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    // Root expanded by default (depth < 1), children not expanded
    let visible = tree::flatten_visible(&tree);
    assert!(visible.len() >= 2, "root + children visible");
}

#[test]
fn test_tree_search() {
    let processes = vec![
        make_process(1, "init", None),
        make_process(10, "myapp", Some(1)),
        make_process(20, "other", Some(1)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    let results = tree::search_tree(&tree, "myapp");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].pid, 10);
}

#[test]
fn test_tree_search_by_pid() {
    let processes = vec![
        make_process(1, "init", None),
        make_process(10, "child", Some(1)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    let results = tree::search_tree(&tree, "10");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "child");
}

#[test]
fn test_tree_format_text() {
    let processes = vec![
        make_process(1, "root", None),
        make_process(10, "child", Some(1)),
    ];

    let tree = tree::build_process_tree(&processes, 0);
    let text = tree::format_tree_text(&tree);
    assert!(text.contains("root"));
    assert!(text.contains("child"));
}

#[test]
fn test_tree_empty_processes() {
    let tree = tree::build_process_tree(&[], 0);
    assert!(tree.is_empty());
}
