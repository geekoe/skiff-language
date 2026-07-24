# P5-F136：Package Artifact Identity Golden 闭合

状态：Ready

## 权威设计

- `doc/architecture/package-service-contract-deployment.md`
- 相关条款：`version` 不参与内容 identity；PackageBuildId 表示具体不可变代码 build。

## DAG 与边界

- 节点：C0 机械闭合。
- 前置：P5-F115 及当前 Phase 5 integration checkpoint 已合流。
- 完成后解除：C1 shared service-call stream 审计及 C2 consumer 重验。
- 当前成熟度：Implementation Checkpoint；本任务不得宣称形成预验收或稳定候选。

## 写入范围

- `artifact-identity/src/tests/canonical_compile_contract/package_artifact_identity.rs`
- 若失败证明 production identity projection 与权威设计冲突，立即报告 `TASK_NOT_EXECUTABLE`，不得自行改变公共
  identity 语义或扩大到其他 production owner。

## 完成标准

1. 在当前 integration checkpoint 复现
   `package_artifact_assign_validate_and_golden_identities` 的失败并记录 actual/expected。
2. 用 projection、相邻不变量测试和权威设计证明 actual 值来自当前 canonical facts，不能盲改 expected。
3. 只更新已失效 golden；确认 package/dependency version relabel 不改变 local ABI/build identity。
4. 搜索同一旧 golden 残留并运行聚焦测试。

## 验证与证据

- 风险：中（identity golden，但 production identity 算法不在写入范围）。
- 唯一聚焦验证 owner：本任务开发 Agent。
- 命令：
  `cargo test -p skiff-artifact-identity package_artifact_assign_validate_and_golden_identities -- --exact`
- 必须额外运行包含 version relabel 不变量的同测试模块或等价聚焦 selector。
- 证据只对任务基线及最终提交有效；identity projection、fixture 或 schema 变化即失效。

## Worktree 与提交

- worktree：`/Users/geek/workspace/skiff-p5-f136`
- branch：`codex/p5-f136-package-artifact-identity-golden`
- 从当前 `codex/package-service-phase-05` checkpoint 创建。
- 提交改动，不 push，不操作 stable，不运行完整 gate。
- 这是一次性有界开发会话；启动后五分钟内开始实际修改，否则返回精确 blocker。

