# P5-F445H-E4R2 timeout, catch and owner closure

状态：Ready。E4R 第二波 timeout 叶子；可与 R3/R4 并行。完成后只提供 R5 的 timeout/catch
输入，不代表 E4R/F445H 完成。

## 直接父节点与精确代码状态

- `P5-F445H-E4R1-evaluator-spine-actual-pending-checkpoint-result.md`
- `P5-F445H-E4R0-evaluator-closure-execution-preflight-result.md`

本任务文件完整描述本节点需求。production base 为 R1 implementation
`b1faea534654c2ee2109f444a6cad6b1168b8445`；后续 task/result 文档不改变 production。
R1 已把 timeout statement/expression 两个 root arm 薄转发到唯一 child，并预声明测试 child。

E1 已冻结 current owned execution control、scripted clock、`derive_timeout_child`、
owner-aware checkpoint、`RuntimeError::scope_terminal` 与 internal
`ScopeTerminalCarrier`。不得回改 E1、root dispatch、错误公共契约或 I6 request boundary。

## 唯一写集

Production：

- `runtime/eval/src/eval_context/timeout.rs`

Tests：

- `runtime/eval/src/program_execution/execution_scope_tests/evaluator_timeout.rs`

交付文档：

- 新增 `P5-F445H-E4R2-timeout-catch-owner-closure-result.md`

不得修改：

- `runtime/eval/src/eval_context.rs`、其它 `eval_context` child；
- `execution_scope_tests.rs` module declaration；
- `exceptions.rs`、`error.rs`、E1 scope owner、capability-context；
- concurrent、actual-Pending、stream、host/native/request/I6；
- Cargo、manifest、lockfile或其它任务/result。

## Production 终态

`timeout.rs` 对 `EvalContext` 定义 `pub(super)` async helper，只消费 parent/child context 的值
所有权：

- statement timeout 在 parent context clone 上调用 `derive_timeout_child`，以 child current
  control执行 block，原样返回 body `Flow`；
- expression timeout 同样执行 value并返回 carrier；
- 不跨 `await` 原地替换 parent shared control；
- normal、throw、return、0 ms、最大合法 `u64`、cancel和future drop均恢复 parent current
  scope，不泄漏 child。

child 执行得到 `RuntimeError::ScopeTerminal` 时：

1. 只有 carrier local source/nesting 与本 wrapper child scope 精确匹配，wrapper才物化
   `RuntimeError::UserException`；
2. payload identity 必须是 `std.error.TimeoutError` /
   `PlatformBuiltinErrorIdentity::Timeout`；
3. details 保留 `reason=deadlineExceeded`、`deadlineSource=scope`、
   `deadlineNesting` 与完整 `deadlineSite`；
4. source site、correlation和stack使用当前 timeout wrapper位置；
5. inherited outer/request terminal穿过当前 wrapper和 ordinary catch，不在这里物化；
6. ancestor cancellation 不可 catch；
7. instruction limit等非 scope budget继续走既有 `ExecutionBudgetExceeded`。

不得用 `ordinary_catch_projection()` 猜 owner，也不得把 `ScopeTerminalCarrier` 放入普通 payload、
wire error或 request heap。

## Nested owner 与 catch

必须同时证明：

- inner deadline更早：inner wrapper唯一物化；
- outer deadline更早：terminal穿过 inner，由 outer唯一物化；
- absolute deadline相同：固定只由最外层 owner物化一次；
- local wrapper内部 ordinary `catch<TimeoutError>` 不能提前截获 internal carrier；
- 正确 owner wrapper物化后，外层 ordinary `catch<TimeoutError>` 可以捕获；
- catch 后 parent execution继续，parent current scope恢复；
- inherited request-like deadline不被 local wrapper延长、不物化、不被 ordinary catch；
- ancestor cancel与deadline同一 poll ready时，cancel优先。

ordinary catch 的 production owner预计无需改动。本节点只能通过真实 evaluator/catch路径证明
既有行为；若正确实现需要修改 `exceptions.rs`、`error.rs` 或 E1，必须停止。

## Test-first 与最低矩阵

先在冻结 fail-closed child上新增真实 RED，再实现。selector：

```text
f445h_e4r_timeout
```

listing 和 execution 都必须至少有 **8 个实际 Rust 测试函数**，且包含 statement与expression
真实 root arm、child block/expression、ordinary catch/rethrow，不得直接调用 timeout helper或
直接构造 carrier冒充 wrapper执行。

最低覆盖：

1. statement timeout normal/return 与 parent恢复；
2. expression timeout value 与 child current scope可见；
3. local owner物化、wrapper内 catch miss、外层 catch hit、catch后继续；
4. nested inner-earlier；
5. nested outer-earlier；
6. equal absolute deadline outer-only；
7. inherited/request-like deadline不延长、不物化、不可 ordinary catch；
8. ancestor cancel同刻优先，timeout child scope lifecycle归零；
9. 0 ms、`u64::MAX`、throw和future drop可用参数化补齐，但不能丢失 statement/expression
   两类真实入口。

使用 E1 现有 `ScriptedClock` / `with_execution_clock`，不能依赖 Tokio wall timer偶然调度。
测试必须断言 materialization owner/nesting/site、payload identity、stack/correlation必要字段、
parent scope恢复和所有 child生命周期计数归零。

## 验证

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r2-timeout/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_timeout -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r2-timeout/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked f445h_e4r_timeout -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r2-timeout/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r2-timeout/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录 listing/execution 的实际非零数；少于 8 不算完成。不运行完整 eval、其它 E4R selector、
prepared owner/DB selector、stable、live、network或 MongoDB。

result 必须记录 implementation/result commit、statement/expression入口、nested三种 owner顺序、
catch投影、cancel优先、parent恢复、实际测试数和验证结果。

## 停止条件

出现任一情况立即返回 `TASK_SCOPE_EXPANDED`，不得越界或派子 Agent：

- `ScopeTerminalCarrier::is_owned_by` 无法区分当前 wrapper；
- 正确物化需要改 `error.rs`、`exceptions.rs`、capability-context、公共 error类型或 E1；
- inherited terminal只有等 I6 才能在 evaluator内部保持；
- child helper必须借 outer mutable context跨 `await`；
- 一次有界探查后仍有多个会改变实现方向的未知量。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r2-timeout
branch   codex/p5-f445h-e4r2-timeout
```

不得派子 Agent。先提交 production/tests implementation，再单独提交 result；返回两个 commit、
矩阵、未决问题和 clean worktree。不得 merge、rebase或 push。

风险：高。开发自验收不替代 R5 combined acceptance；R1 root/E1/error identity变化会使证据
失效。
