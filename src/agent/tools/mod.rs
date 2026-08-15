//! agent 内部 tool 实装（proc_help 元 tool 等）。这些 tool 不进 MCP 46 tool
//! handler，仅 agent Layer 0 / Layer 1 可见。

pub mod catalog;
pub mod help;
