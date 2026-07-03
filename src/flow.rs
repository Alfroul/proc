//! v0.12.0：Windows-only ProcessFlow 数据结构 + Schannel 路径 reaper。
//!
//! 本模块承载端到端流的稳定表示，UI / CLI / 录屏共用。所有 Linux eBPF 路径
//! （FlowSource / FlowAggregator / FlowEvent / RawEvent）已在 v0.12 阶段 2 移除。
//!
//! ProcessFlow 在 Windows 上由 [`crate::schannel_etw`] worker drain 的
//! `SniRecord` 直接构造（[`crate::app::App::overlay_flow_sni_schannel`]）。
//! `bytes_out` / `bytes_in` / `remote_addr` / `remote_port` 留空——Schannel
//! event 不给 socket 元数据。

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

/// exit-accounting：进程退出后 flow 还要保留多久（"幽灵 flow" 窗口）。
/// 30s 让用户能看到刚结束的连接；超时后 [`reap_expired_flows`] 把它从
/// `App::flows` 移除。
pub const GHOST_FLOW_TTL: Duration = Duration::from_secs(30);

/// 端到端流：进程 (pid, start_time) → 远端 (addr, port) 的双向流量记录。
///
/// v0.12.0：移除 `source` 字段（Windows-only 后唯一来源是 Schannel ETW）。
/// serde `#[serde(default)]` 让旧录屏（v0.10/v0.11 含 source 字段的 `.prec`）
/// 反序列化时直接忽略未知字段（serde 默认行为），保持向后兼容。
///
/// 不 derive `Default`：`SystemTime` 不实现 `Default`，手动构造更稳。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessFlow {
    /// 进程 PID。
    pub pid: u32,
    /// 进程 start_time（Unix epoch 秒；PID 复用防串，与 `ProcessInfo` 一致）。
    pub start_time: u64,
    /// 进程 comm（由 App 从 sysinfo 补；MVP 可空）。
    pub comm: String,
    /// 本地地址（Schannel event 不提供，留空字符串）。
    pub local_addr: String,
    /// 远端 IPv4 地址字符串（`"1.2.3.4"`）。Schannel 路径留空。
    pub remote_addr: String,
    /// 远端端口。Schannel 路径留 0。
    pub remote_port: u16,
    /// 出向字节数（Schannel 路径留 0）。
    pub bytes_out: u64,
    /// 入向字节数（Schannel 路径留 0）。
    pub bytes_in: u64,
    /// 关联到的 DNS 查询名（去掉 trailing dot；None = 未关联到，不代表可疑）。
    pub dns_name: Option<String>,
    /// TLS ClientHello 直接抓到的 SNI 明文（Schannel ETW event 1793）。
    #[serde(default)]
    pub sni: Option<String>,
    /// 第一次见到该 flow 的时间。
    pub first_seen: SystemTime,
    /// 最后一次见到该 flow 的时间。
    pub last_seen: SystemTime,
    /// 进程退出时间（exit-accounting）。
    pub exit_time: Option<SystemTime>,
}

impl ProcessFlow {
    /// 是否为「幽灵 flow」——进程已退出但还在 [`GHOST_FLOW_TTL`] 保留窗口内。
    /// UI 用此标记加 `👻` 前缀 + 灰色 / 斜体渲染（[`crate::tui::port_table`]）。
    #[must_use]
    pub fn is_ghost(&self) -> bool {
        self.exit_time.is_some()
    }
}

/// 标记 `alive_pids` 集合外所有 flow 的 `exit_time`（exit-accounting 入口）。
///
/// Schannel event 自带 PID 但无进程退出事件。调用方（`App::tick_light_refresh`）
/// 在 heavy refresh 拿到 alive_pids 时调本函数给 dead flow 打 exit_time，
/// 后续 [`reap_expired_flows`] 按 [`GHOST_FLOW_TTL`] 移除。
pub fn mark_dead_flows(flows: &mut [ProcessFlow], alive_pids: &HashSet<u32>, now: SystemTime) {
    for flow in flows.iter_mut() {
        if flow.exit_time.is_none() && !alive_pids.contains(&flow.pid) {
            flow.exit_time.get_or_insert(now);
        }
    }
}

/// 移除 `exit_time + GHOST_FLOW_TTL < now` 的 flow（30s 保留窗口逻辑）。
/// live flow（无 exit_time）永远保留。
pub fn reap_expired_flows(flows: &mut Vec<ProcessFlow>, now: SystemTime) {
    flows.retain(|f| {
        let Some(exit) = f.exit_time else {
            return true;
        };
        let Some(deadline) = exit.checked_add(GHOST_FLOW_TTL) else {
            return true;
        };
        deadline > now
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_flow(pid: u32, sni: &str, ts: SystemTime) -> ProcessFlow {
        ProcessFlow {
            pid,
            start_time: 0,
            comm: String::new(),
            local_addr: String::new(),
            remote_addr: String::new(),
            remote_port: 0,
            bytes_out: 0,
            bytes_in: 0,
            dns_name: None,
            sni: Some(sni.into()),
            first_seen: ts,
            last_seen: ts,
            exit_time: None,
        }
    }

    #[test]
    fn is_ghost_helper_reflects_exit_time() {
        let mut flow = mk_flow(1, "x.com", SystemTime::UNIX_EPOCH);
        assert!(!flow.is_ghost());
        flow.exit_time = Some(SystemTime::UNIX_EPOCH);
        assert!(flow.is_ghost());
    }

    /// mark_dead_flows：pid 不在 alive_pids 时打 exit_time。
    /// 已有 exit_time 的 flow 不被重复打（get_or_insert 语义）。
    #[test]
    fn mark_dead_flows_marks_dead_pids() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let mut flows = vec![
            mk_flow(100, "alive.com", now),
            mk_flow(200, "dead.com", now),
            mk_flow(300, "ghost.com", now),
        ];
        // pid=300 已有 exit_time（被前一次 mark 过）
        flows[2].exit_time = Some(now - Duration::from_secs(60));

        let alive: HashSet<u32> = [100].into_iter().collect();
        mark_dead_flows(&mut flows, &alive, now);

        assert!(flows[0].exit_time.is_none(), "alive pid 不应被 mark");
        assert_eq!(flows[1].exit_time, Some(now), "dead pid 应被打上当前 now");
        assert_eq!(
            flows[2].exit_time,
            Some(now - Duration::from_secs(60)),
            "已有 exit_time 不应被覆盖"
        );
    }

    /// reap_expired_flows：exit_time + 30s < now 的 flow 被移除；
    /// 仍在窗口内的 ghost flow 保留；live flow 保留。
    #[test]
    fn reap_expired_flows_removes_only_expired() {
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let mut flows = vec![
            // 0: live flow → 保留
            mk_flow(100, "live.com", t0),
            // 1: ghost flow，exit + 29s（仍在窗口内）→ 保留
            mk_flow(200, "ghost-recent.com", t0),
            // 2: ghost flow，exit + 31s（已超窗口）→ 移除
            mk_flow(300, "ghost-expired.com", t0),
        ];
        flows[1].exit_time = Some(t0 - Duration::from_secs(29));
        flows[2].exit_time = Some(t0 - Duration::from_secs(31));

        reap_expired_flows(&mut flows, t0);

        assert_eq!(flows.len(), 2, "应保留 live + recent ghost");
        let pids: Vec<u32> = flows.iter().map(|f| f.pid).collect();
        assert!(pids.contains(&100), "live flow 保留");
        assert!(pids.contains(&200), "recent ghost 保留");
        assert!(!pids.contains(&300), "expired ghost 应被 reap");
    }

    /// reap_expired_flows：空 Vec / 全 live 不 panic。
    #[test]
    fn reap_expired_flows_empty_or_all_live_no_panic() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000);
        let mut empty: Vec<ProcessFlow> = Vec::new();
        reap_expired_flows(&mut empty, now);
        assert!(empty.is_empty());

        let mut all_live = vec![mk_flow(1, "a.com", now), mk_flow(2, "b.com", now)];
        reap_expired_flows(&mut all_live, now);
        assert_eq!(all_live.len(), 2, "全 live 不应被 reap");
    }

    /// serde 向后兼容：旧录屏（v0.10/v0.11）含 `source` 字段的 JSON 反序列化
    /// 时直接忽略未知字段（serde 默认行为）。
    #[test]
    fn process_flow_serde_ignores_legacy_source_field() {
        // 用 round-trip 生成的 JSON（含合法 SystemTime 序列化形态），手动插入
        // `source` 字段模拟旧录屏。
        let original = mk_flow(123, "example.com", SystemTime::UNIX_EPOCH);
        let mut value = serde_json::to_value(&original).expect("serialize");
        let obj = value.as_object_mut().expect("object");
        obj.insert(
            "source".to_string(),
            serde_json::Value::String("ebpf".into()),
        );
        let json = serde_json::to_string(&value).expect("re-serialize");
        // serde 默认忽略未知字段（source），不报错。
        let flow: ProcessFlow = serde_json::from_str(&json).expect("serde 应忽略 source 字段");
        assert_eq!(flow.pid, 123);
        assert_eq!(flow.sni.as_deref(), Some("example.com"));
    }
}
