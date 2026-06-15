use std::collections::HashSet;

use crate::collect::SystemSnapshot;

/// 记录当前所有活跃 PID
pub fn snapshot_pids(sys: &SystemSnapshot) -> HashSet<u32> {
    sys.process_cache().keys().copied().collect()
}

/// 对比两个快照，返回 (新增 PID, 死亡 PID)
pub fn diff_snapshots(old: &HashSet<u32>, new: &HashSet<u32>) -> (Vec<u32>, Vec<u32>) {
    let new_pids: Vec<u32> = new.difference(old).copied().collect();
    let dead_pids: Vec<u32> = old.difference(new).copied().collect();
    (new_pids, dead_pids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_detects_new_and_dead() {
        let old: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let new: HashSet<u32> = [2, 3, 4].into_iter().collect();
        let (new_pids, dead_pids) = diff_snapshots(&old, &new);
        assert_eq!(new_pids, vec![4]);
        assert_eq!(dead_pids, vec![1]);
    }

    #[test]
    fn test_diff_no_change() {
        let old: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let new: HashSet<u32> = [1, 2, 3].into_iter().collect();
        let (new_pids, dead_pids) = diff_snapshots(&old, &new);
        assert!(new_pids.is_empty());
        assert!(dead_pids.is_empty());
    }

    #[test]
    fn test_diff_empty_old() {
        let old: HashSet<u32> = HashSet::new();
        let new: HashSet<u32> = [1, 2].into_iter().collect();
        let (new_pids, dead_pids) = diff_snapshots(&old, &new);
        assert_eq!(new_pids.len(), 2);
        assert!(dead_pids.is_empty());
    }

    #[test]
    fn test_diff_empty_new() {
        let old: HashSet<u32> = [1, 2].into_iter().collect();
        let new: HashSet<u32> = HashSet::new();
        let (new_pids, dead_pids) = diff_snapshots(&old, &new);
        assert!(new_pids.is_empty());
        assert_eq!(dead_pids.len(), 2);
    }
}
