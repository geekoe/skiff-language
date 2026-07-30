# P5-F445H-O5A Prepared DB capability seam

状态：Ready。O5 停止后新增的共享接口前置；完成后重发 O5，再启动 O6。

## 直接父节点

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-O5-service-db-prepared-runtime-operation-result.md`

production prerequisite 为 Skiff integration `b6cb8a5d`。本节点只拥有 capability abstraction，
不实现 Mongo/service-db mapping，也不修改 evaluator。

## 生产目标

在 `DbCapabilityStoreApi` / `DbCapabilityStore` 中增加一次性 prepared runtime operation seam：

1. prepare为同步调用，只在调用期间借 caller `RuntimeValue` / `RequestHeap`；
2. 返回的 prepared operation不携带 caller heap/env/evaluator lifetime；
3. owned wait一次性消费 prepared operation，持有 provider-owned request/command状态，并返回一次性
   finalizer或等价 owned completion；
4. finalizer只在 wait结束后同步接收 `&mut RequestHeap`，物化 typed runtime outcome；
5. wait/finalizer不可 Clone、不可重放；drop/error不启动第二次 provider operation；
6. provider错误继续使用既有 `DbCapabilityError`，typed结果精确覆盖 eval可达的
   find-one/find-many/create/update/replace等 runtime路径；
7. transaction/lease raw APIs与 `DbDocument` APIs不变。

当前唯一 implementor尚未接新 seam。为保持这个分阶段 checkpoint可编译，新 trait方法允许提供
明确、稳定、fail-closed的默认 `prepared DB runtime operation is unavailable`；默认实现不得退回
旧 heap-borrowing async方法。O5R必须覆盖该默认后，O6/J1才可接受 production。

既有 `*_runtime(..., heap).await` 暂时保留；本节点不得重写成伪 prepared wrapper，也不得声称
actual-Pending已完成。

精确 Rust enum/trait-object布局由本节点决定，但必须让调用方可以：

```text
let prepared = store.prepare_...( ..., &mut heap, ...)?;
let wait = prepared.into_wait();   // 到这里已不借 heap
let completion = wait.await?;
let value = completion.finalize(&mut heap)?;
```

## Test-first 与验收

先增加 capability-level fake store RED；至少覆盖：

- prepare返回后 caller heap可立即独立 mutation，wait仍存活；
- wait完成前 heap checkpoint/stats/既有节点不变；
- Ready/Pending fake wait都只启动一次；
- completion/finalizer只能消费一次，drop/error不重启；
- finalize才修改 heap，资源失败回滚部分 allocation；
- default implementor明确 fail closed且绝不调用旧 async runtime方法；
- typed one/many/value outcome不会互相混淆；
- raw DB、transaction和lease接口无 diff/行为不回归。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5a-db-capability/build/cargo-target \
  cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5a-db-capability/build/cargo-target \
  cargo test -p skiff-runtime-capability-context --locked --no-fail-fast
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5a-db-capability/build/cargo-target \
  cargo check -p skiff-runtime-capability-context --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5a-db-capability/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数。

## 写集与停止规则

只允许：

- `runtime/capability-context/src/db.rs`
- `runtime/capability-context/src/db/**`
- `runtime/capability-context/src/lib.rs`（仅必要 re-export/module）
- 本 result

不得修改 service-db、eval、Actor、host/native、Cargo manifest或 lockfile。若 type erasure无法在
不借 heap的前提下保留 typed outcome、必须让 evaluator依赖具体 provider、或必须改变 DB
storage/recoverable语义，立即 `TASK_SCOPE_EXPANDED`。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o5a-db-capability
branch   codex/p5-f445h-o5a-db-capability
```

先提交 implementation，再提交
`P5-F445H-O5A-prepared-db-capability-seam-result.md`；最终 clean，不 merge/rebase/push，
不运行 stable/live/network，不派子 Agent。
