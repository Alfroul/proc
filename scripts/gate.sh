#!/usr/bin/env bash
# proc 质量门禁（v0.26 stage 2，ADR-0036 D2）。
#
# 用法：
#   bash scripts/gate.sh          # 快档（默认）：fmt + clippy 双档 + 核心测试子集，目标 < 5 min
#   bash scripts/gate.sh fast     # 同上
#   bash scripts/gate.sh full     # 全档：快档 + 全量回归双档（~15 min，stage 完工 / 合并前）
#
# 设计动机（R2 教训，brainstorm 基线验证异常记录段）：v0.25 stage 3 的 clippy
# 漏检源于「变更落在人工验证之后」——本脚本把 fmt → clippy → test 从人工排序
# 变成脚本强制顺序，任一步失败立即中止。
#
# 防线分层（ADR-0036 D2）：本脚本 + pre-push hook（opt-in，安装：
# git config core.hooksPath .githooks）+ GitHub required checks（主防线，
# 设置说明见 docs/stages/v0.26-stage-2.md「required checks 设置说明」段）。
#
# 快档测试子集 21 binary（stage-1 附录 B 耗时摸底选型 + 本阶段新增
# test_filter_proptest）：单次 cargo 调用串行执行，省 20 次 cargo spawn 开销。
#
# 实测时长（i7-13700HX / 16GB / Win11，2026-08-31）：稳态（无变更）29s；
# src 变更后首跑 ~9.5 min（clippy 双档编译 + 21 binary release thin-LTO
# 链接主导——名单大小非主要变量，砍名单不显著缩短）。
set -euo pipefail
cd "$(dirname "$0")/.."

MODE="${1:-fast}"
if [[ "$MODE" != "fast" && "$MODE" != "full" ]]; then
    echo "usage: bash scripts/gate.sh [fast|full]" >&2
    exit 2
fi

SECONDS=0
step() { printf '\n==> %s\n' "$*"; }

step "[1/4] cargo fmt --check"
cargo fmt --all -- --check

step "[2/4] cargo clippy（默认档）"
cargo clippy --release --all-targets -- -D warnings

step "[3/4] cargo clippy（--features anthropic）"
cargo clippy --release --all-targets --features anthropic -- -D warnings

FAST_TESTS=(
    test_filter_expr
    test_filter_expr_v2
    test_filter_proptest
    test_agent_rag
    test_mcp_server
    test_mcp_v0_17
    test_mcp_v0_25_stage_3
    test_worker_restart
    test_worker_metrics
    test_workers
    test_security
    test_record
    test_replay_search
    test_replay_direction
    test_record_protection
    test_net_flow
    test_dns_log
    test_app_group
    test_monitor
    test_alert
    test_kill_tree
)

step "[4/4] 测试子集（快档 ${#FAST_TESTS[@]} binary）"
test_flags=()
for t in "${FAST_TESTS[@]}"; do
    test_flags+=(--test "$t")
done
cargo test --release -q "${test_flags[@]}"

if [[ "$MODE" == "full" ]]; then
    step "[全档] 全量回归（默认档）"
    cargo test --release -q
    step "[全档] 全量回归（--features anthropic）"
    cargo test --release -q --features anthropic
fi

step "gate [$MODE] 全绿（${SECONDS}s）"
