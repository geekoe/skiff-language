# Phase 7 activation amendment（de facto baseline）

> Status: activated; user-authorized de facto baseline, formal Phase 6 Gate/Acceptance skipped
>
> Authority: [`phase-7-whole-system-closure.md`](../phases/phase-7-whole-system-closure.md) §1 与
> [`phase-7-execution-map.md`](./phase-7-execution-map.md) §1。本 amendment 是激活记录，不是语义 authority；
> 它把 Phase 6 handoff 的各字段以"de facto 基线 + 聚焦验证证据"落账。

## 1. 授权记录

用户明确决定：不运行完整 Phase 6 Gate / detached Acceptance；以合并后 main HEAD 作为 Phase 7 激活的
de facto baseline。此决定记录在此，Phase 7 基于 `62edf78410aa6a26dfb92a26c3a8422d87d5a23b` 开始实施。

Phase 6 聚焦验证（main `62edf7841`，未跑完整 Gate）证据：

| 项 | 结果 |
| --- | --- |
| host `bytecode_vm_phase_6` | 102 passed / 0 failed |
| router `bytecode_vm_phase_6` | 17 passed / 0 failed |
| scheduler `bytecode_scheduler` | 14 passed / 0 failed |
| `cargo fmt --all -- --check` / `git diff --check` | PASS |
| Gate self-tests（Node TAP） | 29/29 |
| runtime / compiler crate DAG 检查 | PASS |
| stream-child 收尾（child stream lifecycle + scheduler port init） | 已 fast-forward 合入 main（`d66f45c44`、`dbf43c895`） |

Phase 6 收尾：25 个 p6 worktree、87 个 `codex/bcvm-p6-*` 分支已按 MAP8 终态清理；`f6-exact-index-r1` 过期 WIP
已确认被 main 取代并丢弃；4 个 stash 均为 Phase 1/5 资产，不触碰。

## 2. MAP7 §1 字段落账

| Field (MAP7 §1) | De facto value | Source |
| --- | --- | --- |
| 1 Phase 6 frozen candidate / Acceptance | `62edf7841`；无正式 receipt（Gate/Acceptance 跳过，用户授权） | 用户决策 + 聚焦验证 |
| 2 upstream closeout baseline | `62edf7841`，main clean，ahead 129 | `git status` |
| 3 active integration | P7P/P7G worktree 从 `62edf7841` 检出；激活 commit 为本文档 | MAP7 §3 |
| 4 cumulative workload API | `phase6WorkloadSpecs(root)` / `phase6WorkloadProvenance(root)` / `phase6BoundedWorkLedger(root)` 已在 `scripts/lib/bytecode-vm-phase-6-contract.mjs` 导出 | preflight §3 |
| 5 capabilities | 12-key capability ledger：service/task-function/task-Actor/interface-local/interface-remote/callback-same-runtime/callback-cross-runtime/Actor/DB/recoverable/request-GC/Actor-compaction。de facto 判定：普通 capability 全部 `accepted`（host/router 矩阵全绿），`callback-cross-runtime` 与 `request-GC`/`Actor-compaction` 为 `disabled/deferred` | 聚焦验证 + Phase 6 契约 §4 |
| 6 observations and memory | per-lane 观察由 Phase 6 观察 schema 承载；`memory_ledger.rs` 为 root aggregate authority（`4da57126c`）；request-GC/Actor-compaction 保持 disabled/deferred | Phase 6 代码 + 契约 §3.7 |
| 7 bounded work | `phase6BoundedWorkLedger(root)` 已导出，五个 key 均有 canonical spec id | preflight 确认 |
| 8 inherited expected-count residuals | 95 个继承 spec：71 missing / 0 null / 24 integer | preflight §2 探测 |
| 9 identities | 动态从候选路径读取；不 pin literal | 契约 §5.3 |
| 10 write owners | P7P proof carriers；P7G Gate；见 MAP7 §3 | 本文 §3 |
| 11 evidence epoch | `P7-E0` | 本文 |

## 3. 激活的 lane 与 worktree

| Lane | Branch | Worktree | 写集 | 首个可观察 status |
| --- | --- | --- | --- | --- |
| P7P whole-system proof | `codex/bcvm-p7-p7p-r1` | `/Users/geek/workspace/skiff-bcvm-p7-p7p-r1` | MAP7 §3 P7P 行 | 一个真实 HTTP 或 ledger-selected whole-system 场景可执行断言 |
| P7G Gate | `codex/bcvm-p7-p7g-r1` | `/Users/geek/workspace/skiff-bcvm-p7-p7g-r1` | MAP7 §3 P7G 行 | 受控 early-red + 依赖 BLOCKED + 独立 PASS/fresh receipt 全 checked |

Cargo：两个 lane 共用单 epoch lease `/tmp/skiff-bcvm-p7-r1-cargo.lockdir` 与共享 target
`/Users/geek/workspace/.skiff-cargo-target`，串行执行；`cargo test` 幂等 `--no-fail-fast`。

## 4. 与正式 Phase 6 accepted 的偏差（记录在案）

- 无 `results/phase-6.md`（未写 accepted result）。Phase 6 当前状态是"编码完成 + 聚焦验证全绿 + 已合入 main + 已清理"，
  未经过 formal Gate/Acceptance。
- `phase6WorkloadSpecs(root)` 的 spec/provenance catalog digest 尚未生成（preflight G-3），Phase 7 P7G 需自行
  deterministic 派生或向 Phase 6 索要。
- 若后续要补正式 Phase 6 closeout，需要重跑完整 Gate 并另写 result；当前按用户授权以 de facto 处理。