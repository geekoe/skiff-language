# linked-type-plan 重构 Phase 0 — 基线锁定 Leaf Task

日期：2026-08-03
状态：execution leaf（一次性有界会话）
Agent：`/root/phase0_type_plan_baseline`
集成目标：`/root/integrate_type_plan`

## 引用链

- 直接父节点（权威设计文档）：
  `doc/implementation/linked-type-plan-refactor.md`（状态：定稿；Phase 0 见
  §5「批次 1」，验收见该节；执行批次不改变设计语义）。
- 仓库规则：`AGENTS.md`（测试入口、聚焦测试命令、行数门禁
  `scripts/check-rust-file-lines.mjs`、保持改动聚焦）。

## DAG 节点

- 本节点：Phase 0（基线锁定 + 调用面审计 + 差分测试）。
- 前置节点：权威设计文档定稿（2026-08-03，无阻塞性待决策）。
- 被解除节点：Phase 0 验收后解除 Phase 1–3（批次 2，同一 crate 串行实现）
  的基线前提；Phase 4–5（批次 3）在本节点审计清单落库前不得启动。
- 本节点不改变任何设计语义；对设计文档 2.3/4.4 只允许事实性修正
  （行号/数量/调用点），与设计语义冲突的事实不在文档修改，上报主 Agent。

## 写入范围

可写：

- `runtime/linked-type-plan/src/type_plan.rs`（仅新增
  `#[cfg(all(test, feature = "test-support"))]` 差分测试 mod
  `differential_legacy_json_baseline_tests`；不触碰生产代码）。
- `doc/implementation/linked-type-plan-refactor-phase0-leaf.md`（本文件）。
- `doc/implementation/linked-type-plan-refactor-phase0-result.md`（结果文档：
  调用面审计、基线证据、纵向探针清单、自验收矩阵）。
- `doc/implementation/linked-type-plan-refactor.md`（仅事实性修正
  2.3/4.4 的行号/数量/调用点，若审计发现偏差；当前预检结论与文档一致，
  预计无需修改）。

禁止：

- 任何生产行为变更；`type_plan.rs` 行数不得超过 3151
  （`scripts/check-rust-file-lines.mjs`，MAX_FILE_LINES=3151，无白名单）。
- 共享主 worktree `/Users/geek/workspace/skiff` 的写入（branch/commit/
  checkout/reset）；其中 5 个无关未提交文件一律不碰。
- 全量 `pnpm verify`；不 push；不跑 chat smoke；不操作 stable instance /
  stable Mongo / PM2 / 4004-4007。

## 非目标（Phase 0 不做）

- 不搬迁任何生产函数/模块；不建立 `RuntimeBuiltinShape` /
  `PlanInputView`；不做 trait 私有化；不删 `#[allow(dead_code)]`。
- 不合并 legacy `from_descriptor`；不改 depth 语义；不引入 fallback。

## 可执行完成标准（对应权威文档 §5 Phase 0 验收）

1. 调用面审计：`runtime/linked-type-plan` 包外部对
   `RuntimeTypePlanLinkedExt` / `RuntimeRecoverableExpectedTypePlanLinkedExt`
   全部方法的调用点清单，含 `runtime/driver/value_codec/type_descriptor.rs`
   临时 adapter；结果写入 result 文档，与设计文档 2.3 表逐项核对。
2. 差分测试（`cargo test -p skiff-runtime-linked-type-plan --features
   test-support` 可编译且通过）：覆盖结构 builtin（Array/Map/Stream）、
   Record/Union/Nullable/Literal、Address 解析的 descriptor、
   TypeParam 替换、depth-32 截断、recoverable expected；语义本来就不同的
   输入列为预期差异并注释，不强行断言相等。
3. 基线证据：改动前聚焦测试结果 + 含新差分测试的结果，按
   「层级 | 命令 | owner | commit/代码状态 | 结果 | 覆盖范围」记录。
4. 纵向探针清单：`runtime/driver/eval/tests` 中覆盖 `from_linked` /
   `from_linked_nested_ref` / recoverable 的端到端测试（文件 + 测试名）；
   baseline 若存在缺口（如 driver/eval/tests 无 recoverable 直测），如实
   记录并给出最近可替代探针，不得虚构。
5. 自验收矩阵（设计条款 | 代码证据 file:line | 反向搜索证据 | 测试命令）
   写入 result 文档；低风险任务仍须跑通新差分测试与既有 crate 测试；
   未跑完整 runtime 套件时标注「聚焦验证」，不得声称全绿。

## 聚焦验证命令

```bash
cargo test -p skiff-runtime-linked-type-plan                       # 既有测试基线
cargo test -p skiff-runtime-linked-type-plan --features test-support   # 含差分测试
node scripts/check-rust-file-lines.mjs                             # 文件行数门禁
```

受影响依赖方的聚焦用例（可选，按时间允许）：

```bash
cargo test -p skiff-runtime-eval linked_type_plan
cargo test -p skiff-runtime-driver-eval --tests program_execution
```

## 证据有效 commit

- Baseline：`c46d65b1`（`refactor/type-plan-phase0` 分支基于此）。
- 本节点交付 commit：`test(linked-type-plan): add phase 0 baseline
  differential tests`（仅含上述可写文件）。

## 交接

完成后向 `/root/integrate_type_plan` 报告：branch、worktree 路径、
commit/tree、实际写集、自验收矩阵、聚焦验证命令与结果、纵向探针清单
路径；最终答复同步主 Agent，并列出需要主 Agent 决策的事实冲突或未知量。
