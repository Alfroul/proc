//! ToolRegistry — Tool 注册中心两层架构（ADR-0030 D2，brainstorm 决策 3 拍板：
//! HashMap 单一数据源 + 多视图）。
//!
//! - Layer 0：4 个 entry tool 默认暴露（proc_ls / proc_metrics_system /
//!   proc_inspect / proc_help，~600 token）
//! - Layer 1：通过 `proc_help(category)` 元 tool 动态发现剩余 42 个 tool schema
//!
//! 单一数据源（`HashMap<String, ToolSchema>`），entry / category 视图只决定
//! 「本次 LLM 调用传哪些」，不需要动态修改 tools map 本身。

use std::collections::HashMap;

use super::types::{ToolCategory, ToolSchema};

/// Layer 0 默认 entry tool 名单（顺序即注入顺序）。
pub const ENTRY_TOOL_NAMES: [&str; 4] = [
    "proc_ls",
    "proc_metrics_system",
    "proc_inspect",
    "proc_help",
];

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolSchema>,
    category_index: HashMap<ToolCategory, Vec<String>>,
}

impl ToolRegistry {
    /// 构造空 registry（stage 2 落地 46 tool 批量注册）。
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册一个 tool（同时维护 category 索引；entry tool 名单见
    /// [`ENTRY_TOOL_NAMES`] 常量，不入索引）。
    pub fn register(&mut self, schema: ToolSchema) {
        self.category_index
            .entry(schema.category)
            .or_default()
            .push(schema.name.clone());
        self.tools.insert(schema.name.clone(), schema);
    }

    /// Layer 0：返回 entry tool（未注册的跳过）。
    pub fn entry_tools(&self) -> Vec<&ToolSchema> {
        ENTRY_TOOL_NAMES
            .iter()
            .filter_map(|name| self.tools.get(*name))
            .collect()
    }

    /// Layer 1：按 category 返回 tools（None = 全部非 entry tool）。
    pub fn tools_by_category(&self, cat: Option<ToolCategory>) -> Vec<&ToolSchema> {
        match cat {
            Some(c) => self
                .category_index
                .get(&c)
                .map(|names| names.iter().filter_map(|n| self.tools.get(n)).collect())
                .unwrap_or_default(),
            None => self
                .tools
                .values()
                .filter(|t| !ENTRY_TOOL_NAMES.contains(&t.name.as_str()))
                .collect(),
        }
    }

    /// 单个查。
    pub fn get(&self, name: &str) -> Option<&ToolSchema> {
        self.tools.get(name)
    }

    /// token 预算监控：给定 tool 名单的 schema 估算 token 总和。
    pub fn total_tokens(&self, names: &[String]) -> usize {
        names
            .iter()
            .filter_map(|n| self.tools.get(n))
            .map(|t| t.estimated_tokens)
            .sum()
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}
