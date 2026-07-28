# P5-F445H-I7-P0R1 official packages contextual-value source migration

状态：`IN_PROGRESS`。

本节点是 P0 official-package gate 首个 blocker 的低风险 repo-local source compatibility leaf。
直接父节点：

- `P5-F445H-I7-P0-official-packages-prepared-consumer-gate-result.md`
- `P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`

父节点继续追溯唯一权威设计。完成本 leaf 只解除 prepared official-package consumer regate；
不直接完成 P0，也不解除 Agine A 或最终 J。

## 1. Exact repository identity and owners

| 项 | 值 |
| --- | --- |
| Skiff grammar/contracts | `54fb087f122c53aed5c017260c7bca43e2b54404` / `008d3a05927cdf845004db980d1b46de263612be` |
| package repo | `/Users/geek/workspace/skiff-packages` |
| package baseline | `19cfab5dfc827450d37e1a103d21f31f8effa4f0` / `44081bd0498919086c13adea97c07722cb768352` |
| target integration branch | `codex/package-service-phase-05` |
| package integration owner | `/root/phase05_packages_integration_steward` |
| Skiff task/result owner | `/root/phase05_integration_steward` |

`skiff-packages` 没有 `AGENTS.md` 或 Phase 05 task-document owner/directory。package commit只包含
source/tests；本 Skiff task 是唯一执行合同，不在 package repo 创建竞争文档层级。

## 2. Preflight facts and exact write owner

current contextual `value { ... }` grammar与以下 private numeric helper parameter 名冲突：

```text
http-session/session.skiff:157
http-session/session.skiff:174
aliyunoss/aliyunoss.skiff:87
```

三个 site 都使用：

```text
if value.round() != value {
```

whole-tree token scan没有发现其它冲突。其余 bare `value {` 是 registry 中八个 canonical
`db transaction value {` expression；`target.value {` 是 member access，不是 value-primary
collision。

唯一 production write set：

```text
http-session/session.skiff
aliyunoss/aliyunoss.skiff
```

只允许重命名 private parameter及其局部引用，不改变 public package API、数值检查、控制流或输出。

## 3. Non-goals and stop conditions

禁止修改 Skiff parser/grammar、public API、package manifests、registry、tests/runner、
stable/live/network/MongoDB或其它 repo。禁止为旧 source增加 parser兼容。

若 compile 暴露 shared/public semantics、需要 manifest/runner/test fixture变化，或不能用局部
private rename闭合，返回 `TASK_SCOPE_EXPANDED`，不得吞并新 owner。

## 4. Verification and handoff

本 leaf 是以下证据的唯一 owner：

- affected official packages 的 offline/non-live list、compile与test；
- exact contextual-token reverse scan；
- format/check与 `git diff --check`；
- 最终 package branch/worktree clean。

implementation完成后把 branch/worktree、commit/tree、实际写集和验证证据直接交给
`/root/phase05_packages_integration_steward`，并向 Skiff文档 owner交结构化 result。package
integration owner串行合入并清理一级 worktree/branch；不得写 main、rebase或push。

