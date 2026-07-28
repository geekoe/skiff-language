# P5-F445H-E4R8 activation wait stack-shape closure

状态：Ready。关闭 activation-relative unary wait进入generic actual-Pending链时的默认线程栈
blocker。与E4R7 test-only修复并行；两者合流后才运行一次完整gate。

## 直接父节点与冻结根因

- `P5-F445H-E4R8-service-error-consumer-stack-preflight-result.md`
- `P5-F445H-E4R6-callback-stack-shape-closure-result.md`
- `P5-F445H-E4R4-current-scope-stream-activation-closure-result.md`

当前production候选为 `464a3319b153527d5d33093d52ea6af97b6f997b`。preflight已证明：

- `PreparedActivationRelativeServiceCall::wait` concrete future约3776 B；
-未装箱进入generic链后相关state约21/31/41 KiB；
- exact service-error consumer默认和单线程均 `SIGABRT`；
- 2.421875 MiB失败、2.4375 MiB通过，属于有限stack shape而非递归；
-首个linked-public first-Ready unary case尚未返回就触发；
- public/internal/unlinked catch、service-error import和fixture没有递归。

唯一owner为 `EvalContext::eval_activation_relative_service_call`。

## 唯一写集

Production：

- `runtime/eval/src/eval_context/actual_pending/activation.rs`

交付文档：

- 新增 `P5-F445H-E4R8-activation-wait-stack-shape-closure-result.md`

禁止修改：

- prepared activation/provider owner；
- service error channel、catch、tests/fixtures；
-通用 `actual_pending.rs` / E3；
-Cargo/manifest/lockfile、公共 API和其它文档。

## 精确实现与语义

只在 `eval_activation_relative_service_call` 中，把 `operation.wait()` 在传给
`await_actual_pending` 前放入private pinned box：

```rust
let wait = Box::pin(operation.wait());
let completed = self.await_actual_pending(wait).await?;
```

随后仍调用同一个 `completed.finalize(self)`。

必须保持：

- target/contract解析和argument prepare只执行一次；
-同一个unary invocation只wait/finalize一次；
- first-Ready不释放Actor segment；
-真实first-Pending才释放，并在finalize前reacquire/fence；
- provider failure先固化一次、caller import一次；
- linked public exact catch命中；
- unlinked public catch miss且fixed bytes不变；
- private provider error固定为 `std.service.InternalError`并可exact catch；
- rethrow source/stack/correlation与failure atomicity不变；
-不提高栈、不改test attribute、不拆/ignore断言。

若call-site boxing不足或需要唯一写集外改动，返回 `TASK_SCOPE_EXPANDED`。

## 验证

不新增测试。所有命令显式清除 `RUST_MIN_STACK` / `RUSTFLAGS`，使用独立target：

```bash
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib \
  ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses \
  -- --exact --nocapture
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib \
  ordinary_exact_public_and_internal_catches_hit_while_unlinked_catch_misses \
  -- --exact --nocapture --test-threads=1
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib \
  restricted_service_diagnostic_ordinary_exports_before_provider_heap_drop \
  -- --exact --nocapture --test-threads=1
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib \
  f445h_e4r_stream_activation_unary -- --nocapture --test-threads=1
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib \
  assembly_execution::ordinary::tests::service_error_consumer \
  -- --nocapture --test-threads=1
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_spine -- --list
env -u RUST_MIN_STACK -u RUSTFLAGS \
  CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo test -p skiff-runtime-eval --locked --lib f445h_e4r_spine -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo check -p skiff-runtime-eval --tests --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-e4r8-fix/build/cargo-target \
  cargo fmt --check
git diff --check
```

必须为：

-目标exact默认与单线程各1/1；
- public对照1/1；
- activation unary selector非零全绿；
- service-error consumer模块5/5；
- spine 23 listed、23/23；
- check/fmt/diff通过。

不运行完整lib/eval、combined、stable、live、network或 MongoDB。完整lib只在E4R7/E4R8合流后
由新gate owner运行一次。

result记录implementation/result commit、唯一diff、默认环境、各selector实际数量、语义保持、
check/fmt/diff、未决问题和clean状态。

## Worktree 与交付

```text
worktree /Users/geek/workspace/skiff-p5-f445h-e4r8-fix
branch   codex/p5-f445h-e4r8-fix
```

不得派子 Agent。先提交production fix，再单独提交result；返回两个commit、计数和clean
worktree。不得 merge、rebase或 push。

风险：高。此节点只改变private future内存布局，不得改变service错误/catch语义。
