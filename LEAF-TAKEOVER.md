# LEAF-TAKEOVER: fix/assembly-admission-execution-fixture（2026-08-02）

父节点：`.task-contracts/HANDOFF-20260802.md` TODO-1 /
`.task-contracts/TAKEOVER-20260802.md` §5.4 第 1 项。
仓库：`/Users/geek/workspace/skiff`；worktree：
`/Users/geek/workspace/skiff-assembly-admission-fix`；分支：
`fix/assembly-admission-execution-fixture`；基线：`6ec0655a`（main HEAD，clean）。

## 任务与结论

让 `cargo test -p skiff-runtime-host --lib -- loader::assembly_admission::tests::execution`
全绿（原 20 失败 / 2 通过）。**已完成**：22/22 通过；全 crate 399/399 通过；
`cargo fmt --check -p skiff-runtime-host` 通过。

## 根因（main dirty 侧 linker/fixture 组合，非 spawned-actor 分支引入）

1. 主工作树合入的 linker 语义（`call_semantics.rs` 的
   `canonical_package_interface_abi_id`）要求每个被链接的 interface 在
   `package_local_abi.implementation_symbols` 中存在“唯一 implementation package
   interface export at exact owner coordinate”，且 `implementation_links.types`
   同步带 source-path 键。执行 fixture（`tests/execution/artifacts.rs` 的
   `implementation_package`）把 `implementation_symbols` 留空、`implementation_links`
   只按 stable schema key 建链 → 20 个用例在 candidate link 阶段失败
   （`unresolved unique implementation package interface export ... example.phase-four-consumer`）。
2. 修掉上述缺口后，剩余 5 个回调用例暴露跨包 interface 拼写问题：consumer 文件把所有
   callback 引用写成 `ServiceSymbol`（本包局部拼写），linker 按“当前包”解析 owner →
   consumer/ provider 两侧 canonical receiver 身份不一致（expected consumer
   packageSymbol vs got provider packageSymbol / serviceSymbol）。生产语义是 interface
   由 provider 包声明，consumer 通过 package dependency 的 `PackageSymbol` 拼写引用。

## 修复内容（仅测试 fixture，不动生产代码）

`runtime/host/src/loader/assembly_admission/tests/execution/artifacts.rs`：

- `implementation_package` 现在为文件里每个 type declaration 投影
  `implementation_symbols` + `implementation_links.types[source_path]`，与
  compiler `project_implementation_types` 的 canonical 形态一致（含
  `type:{package_id}:top-level:{source_path}` local_type_id）。
- 新增 `provider_callback_interface_ref(provider_abi)`（consumer 侧 callback 拼写：
  `PackageSymbol{Dependency(providerPackage), phase_four.implementation.CallbackProbe}`）
  与 `canonical_callback_interface_ref[_for](abi)`（运行时 carrier 使用的 canonical
  `PackageId` 拼写）。consumer 仅当 behavior 为 `InvokeCallback` /
  `ConsumeCallbackStream`（即合同真有 callback）时使用 provider-owned 拼写；
  其余行为保持局部拼写。
- `implementation_file` / `implementation_package` 增加 `provider_abi: Option<&str>`，
  `ProjectedFixture` 在 provider 包构建后取 `local_abi_identity` 传给 consumer
  文件/包构建（顺序原已满足）。

`runtime/host/src/loader/assembly_admission/tests/execution/scenario.rs`：

- `TypedExecutionFixture` 增加 `callback_interface_id`：callback 合同取
  provider-owned canonical id，非 callback 合同取 consumer-owned canonical id；
  手动构造的 `CallbackCapabilityCarrier` 用该 id，与 checkpoint executable
  （addr 2）链接后的 receiver 一致。

## 证据

- selector：`cargo test -p skiff-runtime-host --lib -- loader::assembly_admission::tests::execution`
  → 22 passed / 0 failed（worktree 独立 `build/cargo-target`）。
- 全 crate：`cargo test -p skiff-runtime-host --lib` → 399 passed / 0 failed。
  一次全量跑出现过 `typed_execution_service_stream_request_cancel_cleans_provider_and_isolates_peer`
  `Elapsed(())`（tokio timeout），单独复跑 3/3 通过、二次全量 399/399 通过，
  判定为负载导致的既有 flake，非本改动引入。
- `cargo fmt --check -p skiff-runtime-host` 通过。

## 写集

- `runtime/host/src/loader/assembly_admission/tests/execution/artifacts.rs`
  （+248/-47 约，含 fmt）
- `runtime/host/src/loader/assembly_admission/tests/execution/scenario.rs`（+7/-3 约）
- 本 LEAF 文档。

## 交接

- 分支 `fix/assembly-admission-execution-fixture`；worktree
  `/Users/geek/workspace/skiff-assembly-admission-fix`。
- 下一步：skiff 集成 Agent 合入 main（rebase 后 merge / fast-forward），复跑 selector
  探针，删除 worktree 与已合并分支；随后可跑完整 Rust gate。
