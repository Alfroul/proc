//! GBNF grammar（ADR-0030 D7，brainstorm 决策 8 拍板：`include_str!` 编译时
//! 嵌入 binary，运行时不依赖外部文件路径）。
//!
//! 约束 Gemma 4 E2B 的 tool call 输出为合法 JSON（E2B 偶尔输出乱码 JSON 的
//! 保命手段）。仅 LlamaCppProvider 路径使用（OpenAI `grammar` 字段）。

pub const TOOL_CALL_GRAMMAR: &str = include_str!("tool_call.gbnf");
