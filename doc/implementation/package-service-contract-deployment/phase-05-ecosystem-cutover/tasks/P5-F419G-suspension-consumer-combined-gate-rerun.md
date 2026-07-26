# P5-F419G Suspension consumer combined gate rerun

状态：Ready（N1 / N2 / N3 合流门禁，只读代码）。

## 直接父节点

- `P5-F419A-suspension-consumer-combined-gate-result.md`
- `P5-F419D-suspension-compiler-current-fixture-repair-result.md`
- `P5-F419F-service-error-public-link-reverse-lookup-result.md`

F419A 已证明三层 production 实现与静态终态正确，但被 compiler/runtime 的 current fixture
阻断。F419D 修复 compiler fixture；F419E 修复 runtime fixture，F419F 又修复其中暴露的
service-error public-link production 缺陷。本节点只在新的 exact candidate 上重跑 F419A
联合门禁，决定是否解除 F420。

## 精确候选与边界

- combined candidate：
  `d419518ae5195a5c41f50ce2c63b3622b575da45`；
- tree：
  `4bad9d99dc6fe6d2b3493d8ce0eeab3cb26c21ec`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

启动时必须证明 candidate 为 HEAD ancestor、tree 匹配且 F415 仍为 ancestor。

production / test 代码完全只读。唯一允许写入：

```text
本任务 result
```

不得修复失败、修改 fixture、merge/rebase/push、访问 stable/live 或派子 Agent。任何失败只记录
原始命令、首错、最小 owner 与是否阻断 F420；不得把静态计数冒充实际 listing。

## 必跑门禁

所有 Cargo 命令使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

每个 selector 先运行对应 `-- --list`，再执行：

```bash
# F417 compiler
cargo test --locked -p skiff-compiler-core package_interface
cargo test --locked -p skiff-compiler-source callable_effects
cargo test --locked -p skiff-compiler-lowering suspend
cargo test --locked -p skiff-compiler-projection package_artifact
cargo test --locked -p skiff-compiler-compiled --lib
cargo test --locked -p skiff-compiler-contract --lib
cargo test --locked -p skiff-compiler --test service_conformance
cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation

# F418 deployment
cargo test --locked -p skiff-deployment projection
cargo test --locked -p skiff-deployment storage
cargo test --locked -p skiff-deployment assembly

# F419 runtime
cargo test --locked -p skiff-runtime-capability-context execution_control
cargo test --locked -p skiff-runtime-request execution_budget
cargo test --locked -p skiff-runtime-model callback_projection
cargo test --locked -p skiff-runtime-eval assembly_execution
cargo test --locked -p skiff-runtime-native callback_adapter
cargo test --locked -p skiff-runtime-linker assembly
cargo test --locked -p skiff-runtime-loader runtime_assembly
cargo test --locked -p skiff-runtime-host assembly_admission

# Supplemental integration-risk probes
cargo test --locked -p skiff-artifact-identity public_instance
cargo test --locked -p skiff-runtime-eval --lib

# Combined compile
cargo check --locked \
  -p skiff-compiler \
  -p skiff-deployment \
  -p skiff-runtime-capability-context \
  -p skiff-runtime-request \
  -p skiff-runtime-model \
  -p skiff-runtime-eval \
  -p skiff-runtime-native \
  -p skiff-runtime-linker \
  -p skiff-runtime-loader \
  -p skiff-runtime-host

cargo fmt --all -- --check
git diff --check
```

预期重点计数：

```text
compiler: 5 / 85 / 2 / 63 / 6 / 7 / 14 / 2
deployment: 20 / 13 / 20
runtime: 1 / 6 / 3 / 92 / 7 / 30 / 17 / 31
artifact identity public_instance: 8
full runtime eval: 216
```

计数如因已接受的新增回归产生可解释增量，可以如实记录；不能删选测试来追平旧计数。

## 静态与动态必须确认

1. production 中不存在旧 requirement/protocol owner：
   `BoundaryCancellationContract`、`BoundaryOperationContract.cancellation`、
   `BoundaryOperationContract.may_suspend`、
   `CallbackContractOperationProjection.may_suspend`。
2. concrete `Executable*`、`CallableMayEffects`、`PackageCallableSignature` 与 public/link exact
   equality 保留。
3. F415 collection-name mapping production 链仍在，F419 四个 fixture initializer 仍为
   `4 / 3 / 4 / 2`。
4. unified runtime service lane 只有 `async_stream_cancel`；`ordinary.rs` 只保留 package-direct。
5. 三个 consumer-visible stream deadline probe 与 host task/lease cleanup probe 必须实际执行通过：
   - `provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout`
   - `stream_item_deadline_remains_typed_through_provider_terminal`
   - `terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout`
   - `typed_execution_service_stream_deadline_releases_provider_task_and_lease`
6. F419A 的三组旧失败必须闭合：
   `service_conformance`、FileIR 两项、runtime fixture 八项；typed throw/catch 必须实际通过。
7. current positive producer 仍为 Package v9 / Local ABI v7 / build v10 / ServiceContract 和
   protocol v5；真实 legacy rejection 字符串不计作残留。

## 判定与交付

只有全部 required selector、supplemental full eval、combined check、format/diff、静态检查和四个
deadline/cleanup probe 通过，才判定 `PASS` 并解除 F420。任何失败判定 `FAIL`，本节点不得修复。

写 `P5-F419G-suspension-consumer-combined-gate-rerun-result.md`，记录 exact commit/tree、每个 listing
与执行计数、完整命令矩阵、F419A 旧失败闭合证据、stream deadline、mapping/generation 反搜、
失败（若有）及 F420 是否解除。提交 result 并保持 worktree clean；不 merge/rebase/push。
