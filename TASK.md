# 叶子任务合同：修复 recoverable interface 再编码与 interface 方法不可调度

## 父任务

`/root/skiff_fix_recoverable`（主 Agent 派发，A+B 双基线缺陷）。

## 基线

- Repo：`/Users/geek/workspace/skiff`
- 基线 commit/tree：`1532bd7b`（integration/actor-wave-a HEAD，`git rev-parse` 已验证）
- 分支：`dev/fix-recoverable-interface`
- worktree：`/Users/geek/workspace/wt-skiff-fix-recoverable`

## 写集边界

允许写：

- `runtime/boundary/src/recoverable.rs`
- `runtime/eval/src/recoverable_behavior.rs`
- `runtime/linker/src/assembly_execution/code_linker.rs`
- `runtime/linker/src/linker/file_conversion.rs`
- `runtime/linker/src/linker/link_diagnostics.rs`（仅可见性 `pub(super)` -> `pub(crate)`）
- 测试：`runtime/service-db/src/tests.rs`、`runtime/boundary/src/recoverable/tests.rs`、
  `runtime/boundary/src/binary/tests.rs`（新字段编译必需）、`runtime/eval/src/spawn_ops.rs`、
  `runtime/linker/src/assembly/tests.rs`

禁止：`router/src`、`runtime/host` actor 路径、`runtime/transport`、集成分支
`integration/actor-wave-a`、main、push。共享 target：
`CARGO_TARGET_DIR=/Users/geek/workspace/skiff/target`。

## A：解码 interface 值不可再编码

根因：`restore_local_interface_self` 返回的 `concrete_type_identity` 是 durable
`LocalConcreteRestoreKey`（`abi-type:...`），decode 时被写成
`InterfaceCarrier::Local.concrete_type`；而 encode 侧 `entry_for_runtime_table` 按 runtime
concrete key（`linked_type_ref_runtime_key`）匹配，导致 decode 后的值无法再 encode。

修复：

1. `RecoverableRestoredLocalInterfaceSelf` 新增 `runtime_concrete_type_identity` 字段。
2. decode 构造 carrier 时用 `restored.runtime_concrete_type_identity`。
3. eval `restore_local_interface_self` 返回 `entry.runtime_concrete_type_identity`。
4. 同步测试 hook：
   - `runtime/boundary/src/recoverable/tests.rs`：hook 返回 runtime key；encode hook 校验
     carrier runtime key；roundtrip 断言 runtime key 并追加第二次 encode。
   - `runtime/service-db/src/tests.rs`：同上（新增 `TEST_PROVIDER_RUNTIME_IMPL`），
     追加 write→read→write。
   - `runtime/eval/src/spawn_ops.rs`：断言 decoded carrier concrete_type ==
     `linked_type_ref_runtime_key(...)`、`method_table.id()` 指向当前 program table；
     追加 decode 后再次 encode。
   - `runtime/boundary/src/binary/tests.rs`：新字段编译同步。

## B：interface 方法不可调度

根因：`link_method_table` 只 canonicalize `table.interface`，未重算每个 slot 的
`method_abi_id`；普通 interface call 侧重新 canonicalize，dispatch 时拼写不一致。

修复：

1. `link_method_table`：`link_interface` 后对每个 slot 用
   `canonical_linked_interface_method_abi_id(&table.interface, &slot.method_name)` 重算
   `method_abi_id`，method_name 缺失 fail closed。
2. `file_conversion.rs` legacy `linked_interface_method_slot_plan` 同步重算（同样 fail
   closed），返回 `anyhow::Result`。
3. eval `interface_method_slot_from_linked` 防御性重算（method_name 缺失 fail closed）。
4. 测试：linker 层“非 canonical slot id -> link 后 canonical”；eval 层
   “decode 出的 provider 可实际 dispatch”（ToolProvider providerName 探针）。

## 自验收

- `cargo test -p skiff-runtime-boundary -p skiff-runtime-eval -p skiff-runtime-linker -p skiff-runtime-service-db`
  （聚焦相关包；若包名不同按 workspace 实际包名）
- `node scripts/verify.mjs` 基线 36/36 全绿
- 报告精确命令、结果、写集

## 交接

- 集成 Agent：`skiff_integration`（branch/worktree/commit/tree/写集/自验收矩阵/越界声明）
- 同时通知主 Agent
