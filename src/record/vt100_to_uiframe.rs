//! v0.17 主题 F VT100 replay 增强子模块 — VT100 字节流转码 UiFrame。
//!
//! v0.17 阶段 1 Spike 落地：仅含 struct + trait 声明（stub）。
//! 阶段 5 Slice 实装增量解析 + 累积屏幕 buffer + 30 FPS 切片为 UiFrame。
//!
//! 临时转码路径（ADR-0028 / brainstorm 决策 6 方案 a）：
//! - `proc replay <file>` 检测 VT100 文件 → 临时转码到 `<file>.tmp.v3`
//! - 走 v3 Player 路径 → 退出时删临时文件
//! - 不破坏原 VT100 文件，转码失败可回退 VtPlayer 正向 replay
//!
//! 与 v0.6 落地的 [`crate::record::vt100::VtPlayer`] 正向 replay 路径并行——
//! VtPlayer 不做转码，仅正向 replay VT100 字节流。本转换器把 VT100 字节流
//! 增量解析 + 累积屏幕 buffer + 30 FPS 切片为 [`UiFrame`]，让 VT100 录屏
//! 享受 v0.14 落地的 search / 倒放 / 书签全部能力。

use crate::record::UiFrame;

/// VT100 → UiFrame 转换器（stage 5 实装）。
///
/// stage 1 Spike 仅声明 struct + 三方法 stub（返 "v0.17-stage-5 未实装" 错误）。
/// stage 5 Slice 实装：
/// - `feed_bytes`：VT100 字节流增量解析（CSI / SGR / cursor move / clear 全套
///   VT500 序列反序列化），累积屏幕 buffer + 当前 SGR 状态
/// - `snapshot_frame`：定时（30 FPS）切片屏幕 buffer 为 [`UiFrame`]，写入
///   临时 `<file>.tmp.v3` 文件
///
/// 临时转码路径不破坏原 VT100 文件（与 VtPlayer 正向 replay 路径并行），
/// 转码失败可回退 VtPlayer 路径（VT100 字节流损坏时仍走正向 replay）。
pub struct Vt100ToUiFrameConverter {
    // stage 5 加：屏幕 buffer / 当前 SGR 状态 / 30 FPS 切片定时器 / 帧计数器
    _placeholder: (),
}

impl Vt100ToUiFrameConverter {
    /// 创建新转换器（stage 5 实装）。
    #[must_use]
    pub fn new() -> Self {
        Self { _placeholder: () }
    }

    /// 喂 VT100 字节流增量（stage 5 实装增量解析 + 累积屏幕 buffer）。
    ///
    /// stage 1 Spike 返 "v0.17-stage-5 未实装" 错误。stage 5 实装时解析
    /// VT500 序列（CSI / SGR / cursor move / clear）并累积到屏幕 buffer。
    pub fn feed_bytes(&mut self, _bytes: &[u8]) -> Result<(), String> {
        Err("v0.17-stage-5 未实装".to_string())
    }

    /// 切片为 UiFrame（stage 5 实装 30 FPS 定时切片）。
    ///
    /// stage 1 Spike 返 "v0.17-stage-5 未实装" 错误。stage 5 实装时按 30 FPS
    /// 定时切片屏幕 buffer 为 [`UiFrame`]，含当前进程列表 / timestamp /
    /// anomalies（VT100 路径 anomalies 恒空——VT100 字节流不含 anomaly 标记）。
    pub fn snapshot_frame(&self) -> Result<UiFrame, String> {
        Err("v0.17-stage-5 未实装".to_string())
    }
}

impl Default for Vt100ToUiFrameConverter {
    fn default() -> Self {
        Self::new()
    }
}
