//! 进程的网络连接信息（复用 `port_map`，避免重复扫描）。
//!
//! 阶段 13 的 TUI 渲染网络 Tab 时，直接拿这个 Vec 喂给现有的 PortTable 组件。

pub use crate::port_map::find_ports_by_pid as collect_net;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn self_net_returns_ok_even_if_empty() {
        // 自己进程可能没有任何监听端口，但函数本身应返回 Ok。
        let res = collect_net(std::process::id());
        assert!(res.is_ok(), "got {:?}", res);
    }
}
