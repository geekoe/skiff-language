# P5-F445H-O5B Mutable legacy create finalization seam

状态：Ready。O5R 停止后新增的窄 capability owner；完成后再次重发 O5R。

## 直接父节点

- `P5-F445H-O5A-prepared-db-capability-seam-result.md`
- `P5-F445H-O5R-service-db-prepared-runtime-operation-result.md`

production prerequisite 为 Skiff integration `947e310e`。

`DbCapabilityStoreApi::create_runtime` 与 `DbCapabilityStore::create_runtime` 当前仍接收
`&RequestHeap`，而 O5A 唯一 finalizer 形状是
`DbRuntimeFinalizer::finalize(self, &mut RequestHeap)`。这使 O5R 无法让旧入口薄组合到同一个
prepared create完成路径。

DB reference只承诺 insert返回 attached nominal type，没有承诺与输入保持同一 heap object identity；
DB capability architecture明确规定 runtime从业务值/JSON经过普通类型计划解码。因此本节点是内部
Rust capability可变性修正，不改变用户语言语义。Skiff 尚未发布，不保留旧 immutable overload。

## 生产目标

只做以下收敛：

1. 将 `DbCapabilityStoreApi::create_runtime` 的 `heap` 从 `&'a RequestHeap` 改为
   `&'a mut RequestHeap`；
2. 将 `DbCapabilityStore::create_runtime` 的 `heap` 改为 `&mut RequestHeap`并原样转发；
3. 更新 capability-level fake implementor和唯一 production
   `ServiceDbCapabilityStore` 的对应签名；
4. 保持 `ServiceDbStore` / `ServiceDbRuntime` 当前可接受共享 heap的内部实现不变；`&mut`可安全收窄
   为它们当前所需的 `&`。真正的 prepare→wait→finalize组合仍由重发后的 O5R拥有；
5. 不新增 immutable overload、compatibility分支、unsafe cast或第二个 finalizer形状。

其它五个旧 runtime方法、六个 prepared方法、raw DB、transaction、lease、mapping和storage语义
全部不变。

## Test-first 与验收

先在 fake implementor/compile contract中使用新的 mutable签名，证明当前 trait不接受，再修改
production trait。至少证明：

- trait implementor与 wrapper都要求 mutable heap；
- wrapper可以把同一 mutable heap继续交给一次性 `DbRuntimeFinalizer`；
- O5A prepared focused tests数量与语义不变；
- service-db唯一 implementor、eval现有调用方和四个相关 crates同时编译；
- 反向搜索确认旧 `create_runtime` capability签名不再出现 `&RequestHeap`。

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5b-create-heap/build/cargo-target \
  cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5b-create-heap/build/cargo-target \
  cargo check -p skiff-runtime-capability-context -p skiff-runtime-service-db \
    -p skiff-runtime-eval --locked
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5b-create-heap/build/cargo-target \
  cargo fmt --check
git diff --check
```

记录实际测试数；零测试不算证据。

## 写集与停止规则

只允许：

- `runtime/capability-context/src/db.rs`
- `runtime/capability-context/src/db/prepared_runtime_tests.rs`（仅 module声明，如必要）
- `runtime/capability-context/src/db/prepared_runtime_tests/**`
- `runtime/service-db/src/capability.rs`
- 本 result

不得修改 service-db store/runtime/mapping、eval、Actor、native/host、Cargo manifest或 lockfile。
若除了上述签名和 fake/唯一 implementor外还有第二个 production owner，或修正要求改变 DB用户语义，
立即 `TASK_SCOPE_EXPANDED`。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-o5b-create-heap
branch   codex/p5-f445h-o5b-create-heap
```

先提交 implementation，再单独提交
`P5-F445H-O5B-mutable-legacy-create-finalization-seam-result.md`。最终 clean，不
merge/rebase/push，不运行 stable/live/network，不派子 Agent。
