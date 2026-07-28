# P5-F445H-E4R7 stream deadline regression preflight

状态：Ready。E4R6 完整 lib重验暴露的五条 stream deadline失败只读探查。目标是区分旧
fixture/旧期望与 current-scope production缺陷，并冻结唯一修复节点；本任务不修代码。

## 直接父节点与冻结事实

- `P5-F445H-E4R6-callback-stack-shape-closure-result.md`
- `P5-F445H-E4R4-current-scope-stream-activation-closure-result.md`
- `P5-F445H-E4R5-combined-integration-acceptance-result.md`

当前候选 commit为 `464a3319b153527d5d33093d52ea6af97b6f997b`。callback blocker已修复；
R4 focused 22/22和combined 5/5历史证据有效，但串行完整 lib暴露五条既有
`async_stream_cancel` tests失败：

- `pending_provider_unary_wakes_from_deadline_and_cancels_provider_request`
- `provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout`
- `stream_item_deadline_remains_typed_through_provider_terminal`
- `stream_terminal_item_and_publication_deadlines_remain_typed`
- `terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout`

失败分别表现为缺少旧 `ExecutionBudgetExceeded(DeadlineExceeded)`、得到 `Cancelled`、得到
`End`或typed deadline匹配失败。

## 角色与唯一写集

唯一允许写入：

- 新增 `P5-F445H-E4R7-stream-deadline-regression-preflight-result.md`

production、tests、fixture、Cargo/manifest/lockfile及其它文档只读。可以把日志放在ignored
`build/`，不得临时patch源码或派子 Agent。

## 必须回答

对五条test逐项确定：

1. 使用的 `ExecutionControl`/fixture是否提供调用时 current `ExecutionScope`；
2. deadline来自 legacy `deadline()`、request scope、local child scope还是scripted clock；
3. R4后合法结果应为 internal `ScopeTerminalCarrier`、provider typed timeout、cancel还是End；
4. failure来自旧fixture未提供 current scope、旧期望仍断言generic budget error、真实
   cancellation/deadline竞态，还是 production current-scope传播错误；
5. 五条是否同一根因/同一owner，还是必须拆成多个节点；
6. 最小修复写集、RED/GREEN selector与完整lib重验要求。

必须对照 R4新增 `current_scope_tests.rs` 和权威语义：

- 调用时完整 `cancellation_signals()`；
- effective deadline及owner；
- ancestor cancel同刻优先；
- local/inherited deadline保持internal carrier；
- scope不可取得时fail closed，不允许request-root fallback；
-异常只承诺本地cleanup initiation，不承诺远端ack。

不得为了让旧test通过恢复generic request token/deadline fallback。

## 有界诊断

使用独立 target，默认环境运行五条 exact tests；必要时每条最多复跑一次以确认确定性：

```text
/Users/geek/workspace/skiff-p5-f445h-e4r7-preflight/build/cargo-target
```

允许运行：

- 五条精确 `cargo test -p skiff-runtime-eval --locked --lib <name> -- --exact --nocapture`；
- `f445h_e4r_stream -- --list`，仅用于inventory；
-只读 source/diff/search；
- `cargo check --tests --locked`仅在需要确认fixture编译边界时一次。

不得运行完整 lib/eval、combined、stable、live、network或 MongoDB。

## Result要求

状态只能为：

- `READY_FOR_E4R7_FIX`：单一或明确拆分后的原因、owner、写集和验收已冻结；
- `TASK_SCOPE_EXPANDED`：存在公共语义冲突或多个需要用户/新DAG决定的owner；
- `TASK_NOT_EXECUTABLE`：有界探查仍不能形成安全修复。

result记录：

- 当前 commit/tree与clean状态；
- 五条test的exact exit、actual/expected、fixture scope/deadline来源；
- R4语义下正确期望；
- 被排除的fallback/竞态假设；
- production bug与stale test/fixture的精确分类；
-唯一修复owner、允许写集和禁止面；
- RED/GREEN及一次完整lib重验要求；
-是否需要用户决策。

即使结论是纯test fixture修正，也不得本任务实现。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r7-preflight
branch   codex/p5-f445h-e4r7-preflight
```

只提交result；返回commit、状态、逐test根因、修复写集和clean worktree。不得
merge/rebase/push。

风险：高。错误地恢复legacy deadline fallback会破坏R4 current-scope语义。
