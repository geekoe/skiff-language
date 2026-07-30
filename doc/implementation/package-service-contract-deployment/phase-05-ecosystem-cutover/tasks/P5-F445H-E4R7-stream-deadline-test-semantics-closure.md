# P5-F445H-E4R7 stream deadline test semantics closure

状态：Ready。把五条 F419 时代的旧 stream deadline tests迁到 E1/R4 current-scope语义。纯
test-only修正；不改变production。完成后等待E4R8一起进入唯一完整gate。

## 直接父节点与冻结结论

- `P5-F445H-E4R7-stream-deadline-regression-preflight-result.md`
- `P5-F445H-E4R4-current-scope-stream-activation-closure-result.md`

当前production候选为 `464a3319b153527d5d33093d52ea6af97b6f997b`。E4R7 preflight已证明
五条failure共用一个test-only owner：

- fixture均提供真实 request current `ExecutionScope`；
- request deadline必须保持 internal
  `ScopeTerminalCarrier(InheritedDeadlineExceeded)`；
- carrier进入raw stream boundary时投影为内部 `Cancelled`，不能作为ordinary typed payload；
-一条 `End` 来自consumer attach竞态；
- backpressure固定sleep fixture已过期；
- production current-scope传播、cancel priority和cleanup语义正确。

不得恢复generic budget error或request-root fallback。

## 唯一写集

- `runtime/eval/src/assembly_execution/async_stream_cancel.rs` 中
  `#[cfg(test)] mod tests` 内的五条目标tests及其局部test-only helper；
- 新增 `P5-F445H-E4R7-stream-deadline-test-semantics-closure-result.md`

同一Rust文件的非-test production区域禁止修改。还禁止修改：

- `async_stream_cancel/current_scope.rs`及其tests；
- ordinary test runtime/shared fixture；
- error/carrier/stream runtime/capability-context；
- program stream/invocation；
- Cargo/manifest/lockfile和其它文档。

implementation diff必须证明所有源码hunk位于 `#[cfg(test)] mod tests`。

## 五条终态

将五条test改为统一非零 selector `f445h_e4r7_stream_deadline`，保留五个独立实际函数：

1. pending unary：
   - 断言结果为 inherited request `ScopeTerminalCarrier`；
   -精确检查 source/nesting/deadline owner；
   - provider request本地cancel发生。
2. terminal/item/publication helper matrix：
   - provider terminal与terminal publication在raw边界前保留request carrier；
   - item publication在raw stream边界投影为 `StreamRuntimeError::Cancelled`；
   -不期待generic `ExecutionBudgetExceeded`。
3. provider terminal到raw consumer：
   - raw consumer观察内部 `Cancelled`；
   - stream lifetime/本地cleanup精确收束；
   -不伪造provider-origin typed timeout。
4. item publication：
   - 先用手动first poll或明确gate证明consumer已经进入真实Pending；
   - 再触发request deadline；
   -断言raw `Cancelled`，消除registry移除后看到 `End` 的attach竞态。
5. blocked terminal publication：
   - 使用确定性gate/already-expired scope控制buffer/backpressure顺序；
   -不使用固定sleep；
   -先消费buffered item，再观察raw internal `Cancelled`；
   -保留provider request cancel和本地cleanup断言。

测试名可以从旧名称改为上述统一prefix，但不得删除、合并、ignore或把旧exact零匹配冒充成功。

## 验证

使用独立target：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r7-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r7-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r7_stream_deadline -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r7-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_stream -- --list
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r7-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_stream -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r7-fix/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r7-fix/build/cargo-target \
  cargo fmt --check
git diff --check
```

必须为5 listed、5/5、22 listed、22/22。不得运行完整lib/eval或combined；完整gate由E4R7/E4R8
共同合流后的新owner只运行一次。

result记录：

- implementation/result commit；
-五条旧actual/新expected映射；
-确定性gate如何替代attach race/fixed sleep；
-production diff为零的证明；
-实际数量、check/fmt/diff；
-未决问题和clean状态。

若任一test正确迁移需要改production/current-scope/error/shared fixture，返回
`TASK_SCOPE_EXPANDED`，不得越界或派子 Agent。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r7-fix
branch   codex/p5-f445h-e4r7-fix
```

先提交tests，再单独提交result；返回两个commit、5/5与22/22、production diff零和clean
worktree。不得 merge/rebase或 push。

风险：高。错误断言会把internal carrier重新泄漏到ordinary/raw stream语义。
