# P5-F445H-O2 Outbound service and Actor prepared operation

状态：Ready。actual-Pending correction DAG 的 outbound/Actor owner；与 O1、O3–O5并行。

## 直接父节点

- `P5-F445H-E3R-heap-borrowing-actual-pending-preflight-result.md`
- `P5-F445H-E3-actor-concurrent-continuation-bridge-result.md`

production prerequisite 为 Skiff integration `d39ad5b0`。

## 生产目标

### Outbound service / remote interface

把 legacy outbound unary切成明确阶段：

- prepare：解析 dispatch、用 caller heap编码 payload、同步 `start_request`；
- owned wait：只持 context、dispatch、`OutboundRequestLease`和receiver，不借 caller heap/env；
- finalize：resume后 decode/coerce response到 caller heap，并执行既有 stream-sink取消检查；
- cancel/drop：lease complete/cancel exactly once，late response不能进入 caller。

serverStream setup不是当前调用的 suspension point：同步构造接管 lease/receiver的 stream value，
真实等待留给后续 `stream.next()`。

remote interface operation与 service dependency call共用同一 prepared owner，不各造状态机。

### Actor dispatch

把 `dispatch_actor_method` 切成：

- prepare：receiver/method/arity/plan校验，参数编码为 owned invocation request；
- owned wait：只持 Actor capability context与 request；
- finalize：resume后 decode/import response或映射 Actor/cancel错误；
- drop/cancel沿既有 invocation owner exactly once。

API必须让 E4R在 wait存活期间独立访问 caller heap。现有 async入口可暂时作为薄
prepare→wait→finalize组合以保持编译，不得复制协议或保留静态 pre-suspend。

## Test-first 与验收

先写 RED；至少覆盖：

- outbound buffered/立即response与 pending response，副作用只启动一次；
- wait存活时 caller heap可以独立 mutation，finalize前无 caller写入；
- unary normal/error/drop/cancel的 lease settlement与late response隔离；
- serverStream同步 setup不被标记成 external wait，后续 source拥有 lease；
- remote interface和普通 dependency共用相同阶段合同；
- Actor invocation Ready/Pending、error/cancel/drop、stale/replacement语义不回归；
- finalize decode/heap失败保持既有失败原子性。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor/build/cargo-target \
  cargo test -p skiff-runtime-eval service_dispatch -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor/build/cargo-target \
  cargo test -p skiff-runtime-eval actor_dispatch -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor/build/cargo-target \
  cargo check -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数。

## 写集与停止规则

只允许：

- `runtime/eval/src/service_dispatch.rs`
- `runtime/eval/src/service_dispatch/**`
- `runtime/eval/src/actor_dispatch.rs`
- `runtime/eval/src/actor_dispatch/**`
- 本 result

不得修改 `eval_context.rs`、Actor executor/store、assembly、native、host、service-db或manifest。
若 wait仍借 caller heap/env、serverStream被迫预释放、或 lease无法 exactly once，立即
`TASK_SCOPE_EXPANDED`。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o2-outbound-actor
branch   codex/p5-f445h-o2-outbound-actor
```

先提交 implementation，再提交
`P5-F445H-O2-outbound-actor-prepared-operation-result.md`；最终 clean，不
merge/rebase/push，不运行 stable/live/network，不派子 Agent。
