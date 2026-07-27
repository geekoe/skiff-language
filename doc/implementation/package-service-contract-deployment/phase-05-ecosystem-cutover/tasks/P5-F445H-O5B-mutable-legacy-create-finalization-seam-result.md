# P5-F445H-O5B Mutable legacy create finalization seam result

状态：`IMPLEMENTATION_COMPLETE / CAPABILITY_GREEN`。

旧 `create_runtime` capability 入口现已统一要求 `&mut RequestHeap`。trait、公开
`DbCapabilityStore` wrapper、capability-level fake implementor和唯一 production
`ServiceDbCapabilityStore` 使用同一签名；service-db内部 store/runtime仍只消费共享 heap，
没有改变 DB 用户语义。O5R 可以据此让旧 create wrapper 与 prepared create 共用同一个
prepare → owned wait → one-shot finalizer 路径。

## 1. 输入与提交

| 项 | 值 |
| --- | --- |
| 直接父节点 | `P5-F445H-O5A-prepared-db-capability-seam-result.md` |
| 直接父节点 | `P5-F445H-O5R-service-db-prepared-runtime-operation-result.md` |
| production prerequisite | `947e310e` |
| task document / worktree base | `0e213865` |
| implementation | `2f7af3c3` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-o5b-create-heap` |
| branch | `codex/p5-f445h-o5b-create-heap` |

最终 production 写集精确为：

- `runtime/capability-context/src/db.rs`
- `runtime/service-db/src/capability.rs`

测试写集精确为：

- `runtime/capability-context/src/db/prepared_runtime_tests/contract_tests.rs`
- `runtime/capability-context/src/db/prepared_runtime_tests/fake_store/raw_read_api.rs`

没有修改 service-db store/runtime/mapping、eval、Actor、native/host、Cargo manifest 或
lockfile。

## 2. Test-first 证据

先把 capability-level fake implementor 的 `create_runtime` 改为接收
`&mut RequestHeap`，并增加 wrapper复用同一 mutable heap执行一次性 finalizer的测试；此时
production trait保持不动。运行：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-p5-f445h-o5b-create-heap/build/cargo-target \
  cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture
```

结果为预期 RED，exit `101`。两个 fake implementor分别报告一个 `E0053`，都精确指出
implementor 的 `&mut RequestHeap` 与 trait 的 `&RequestHeap` mutability不一致。失败来自目标
capability签名，不是 fixture、依赖或环境。

随后才修改 production trait、wrapper与唯一 production implementor。同一命令转绿，实际执行
`8/8` tests：O5A原有 `7/7` 测试名称和语义保持通过，新增 `1/1`
`prepared_db_legacy_create_wrapper_reuses_mutable_heap_for_finalization`。

## 3. 收敛结果

`DbCapabilityStoreApi::create_runtime` 和 `DbCapabilityStore::create_runtime` 现在都接收 mutable
caller heap。`ServiceDbCapabilityStore` 原样向下转发；Rust自动把 `&mut RequestHeap` 收窄为
`ServiceDbStore::create_runtime` 当前需要的 `&RequestHeap`，所以内部 store/runtime无需改变。

fake legacy wrapper按后继 O5R需要的形状验证 capability合同：

```text
prepare_create_runtime
  -> into_wait
  -> DbRuntimeFinalizer::finalize(same mutable caller heap)
```

测试证明输入 heap node仍然存在、返回值在同一 heap新增一个 node，并且 legacy调用、wait与
finalize计数都恰为一次。没有新增 immutable overload、compatibility分支、unsafe cast或第二个
finalizer形状。

反向搜索确认：

- `DbCapabilityStoreApi` trait签名为 `&'a mut RequestHeap`；
- `DbCapabilityStore` wrapper签名为 `&mut RequestHeap`；
- 两个 fake implementor与唯一 production implementor都为 mutable签名；
- 这些 capability owner中不再存在接收共享 heap的旧 `create_runtime` 签名；
- 全 runtime只有一个 production `DbCapabilityStoreApi` implementor。

## 4. 验证

所有 Cargo 命令使用 worktree独立 target：

```text
/Users/geek/workspace/skiff-p5-f445h-o5b-create-heap/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-capability-context prepared_db -- --nocapture` | PASS：`8/8` focused tests，`52` filtered |
| `cargo check -p skiff-runtime-capability-context -p skiff-runtime-service-db -p skiff-runtime-eval --locked` | PASS；trait、wrapper、唯一 production implementor和现有 eval调用方同时编译 |
| `cargo fmt --check` | PASS |
| `git diff --check` | PASS |

联合 check只出现仓库既有 linker dead-code warning和 eval unreachable-pattern warning，没有新增
错误。没有运行 stable、live、network或真实 MongoDB，也没有 merge、rebase或 push。

## 5. 后继

O5R可以立即重新发出。它仍应只在 service-db owner内覆盖六个 prepared methods，并让六条旧
runtime入口薄组合到同一 prepare/wait/finalizer实现；create不再需要第二套 completion语义。
本节点没有遗留用户决策或范围阻塞。
