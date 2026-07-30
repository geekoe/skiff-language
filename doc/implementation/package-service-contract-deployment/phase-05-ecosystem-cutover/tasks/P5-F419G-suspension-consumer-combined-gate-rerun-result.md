# P5-F419G Suspension consumer combined gate rerun result

状态：**PASS**。N1 / N2 / N3 exact candidate 的 required selector、supplemental full eval、
combined check、format/diff、静态终态与四个 deadline/cleanup probe 全部通过；F419A 的三组旧失败
均已闭合，**F420 解除**。

## 1. 精确候选、启动门禁与只读边界

| 锚点 | commit | tree / 结果 |
| --- | --- | --- |
| combined candidate | `d419518ae5195a5c41f50ce2c63b3622b575da45` | `4bad9d99dc6fe6d2b3493d8ce0eeab3cb26c21ec` |
| accepted F415 | `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` | ancestor PASS |
| gate start HEAD | `6e73c4e8b52ee6281c515770a32d1707c114e59a` | `12999c62171d86f96e287b0893ae6a7932bb7e4d` |

启动证据：

```text
git merge-base --is-ancestor d419518ae5195a5c41f50ce2c63b3622b575da45 HEAD
exit 0

git rev-parse d419518ae5195a5c41f50ce2c63b3622b575da45^{tree}
4bad9d99dc6fe6d2b3493d8ce0eeab3cb26c21ec

git merge-base --is-ancestor 7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d HEAD
exit 0
```

gate start HEAD 只比 candidate 多本任务 task 文档：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F419G-suspension-consumer-combined-gate-rerun.md
```

因此 production / test / fixture 与 exact candidate 相同。启动时 worktree clean。所有 Cargo
listing、test 与 combined check 均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

本任务没有修改 production、test 或 fixture，没有 merge、rebase、push、stable/live、instance、
watch registry 或子 Agent 操作。

## 2. Required listing 与 execution 矩阵

所有 listing 命令均为表中 test command 后追加 `-- --list`。计数来自实际 listing 输出中的
`: test` / `: benchmark` 项；execution 计数来自实际 test harness 结果。

| test command | listing | execution |
| --- | --- | --- |
| `cargo test --locked -p skiff-compiler-core package_interface` | PASS，5 tests / 0 benchmarks | PASS，5 / 0 |
| `cargo test --locked -p skiff-compiler-source callable_effects` | PASS，85 / 0 | PASS，85 / 0 |
| `cargo test --locked -p skiff-compiler-lowering suspend` | PASS，2 / 0 | PASS，2 / 0 |
| `cargo test --locked -p skiff-compiler-projection package_artifact` | PASS，63 / 0 | PASS，63 / 0 |
| `cargo test --locked -p skiff-compiler-compiled --lib` | PASS，6 / 0 | PASS，6 / 0 |
| `cargo test --locked -p skiff-compiler-contract --lib` | PASS，7 / 0 | PASS，7 / 0 |
| `cargo test --locked -p skiff-compiler --test service_conformance` | PASS，14 / 0 | PASS，14 / 0 |
| `cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation` | PASS，2 / 0 | PASS，2 / 0 |
| `cargo test --locked -p skiff-deployment projection` | PASS，20 / 0 | PASS，20 / 0 |
| `cargo test --locked -p skiff-deployment storage` | PASS，13 / 0 | PASS，13 / 0 |
| `cargo test --locked -p skiff-deployment assembly` | PASS，20 / 0 | PASS，20 / 0 |
| `cargo test --locked -p skiff-runtime-capability-context execution_control` | PASS，1 / 0 | PASS，1 / 0 |
| `cargo test --locked -p skiff-runtime-request execution_budget` | PASS，6 / 0 | PASS，6 / 0 |
| `cargo test --locked -p skiff-runtime-model callback_projection` | PASS，3 / 0 | PASS，3 / 0 |
| `cargo test --locked -p skiff-runtime-eval assembly_execution` | PASS，92 / 0 | PASS，92 / 0 |
| `cargo test --locked -p skiff-runtime-native callback_adapter` | PASS，7 / 0 | PASS，7 / 0 |
| `cargo test --locked -p skiff-runtime-linker assembly` | PASS，30 / 0 | PASS，30 / 0 |
| `cargo test --locked -p skiff-runtime-loader runtime_assembly` | PASS，17 / 0 | PASS，17 / 0 |
| `cargo test --locked -p skiff-runtime-host assembly_admission` | PASS，31 / 0 | PASS，31 / 0 |
| `cargo test --locked -p skiff-artifact-identity public_instance` | PASS，8 / 0 | PASS，8 / 0 |
| `cargo test --locked -p skiff-runtime-eval --lib` | PASS，216 / 0 | PASS，216 / 0 |

汇总：

```text
listing commands:   21 PASS / 0 FAIL
execution commands: 21 PASS / 0 FAIL
listing occurrences: 648 tests / 0 benchmarks
execution occurrences: 648 passed / 0 failed

compiler: 5 / 85 / 2 / 63 / 6 / 7 / 14 / 2
deployment: 20 / 13 / 20
runtime: 1 / 6 / 3 / 92 / 7 / 30 / 17 / 31
artifact identity public_instance: 8
full runtime eval: 216
```

### 环境中断与精确重跑

首轮 listing 的前 18 项实际成功后，本机磁盘只剩约 `116 MiB`。第 19 项 host 与第 20 项
artifact identity 在共享 target 写 `rmeta` 时收到：

```text
No space left on device (os error 28)
```

第 21 项也因同一原因无法创建仓库外临时日志。协调者随后清理了一个已经被 F411 取代、且干净的旧
F386 worktree，释放约 `7.3 GiB`；本任务没有清理共享 target 或其他任务文件。候选 worktree 全程
clean。受影响的第 19、20、21 项 listing 随即从原命令完整重跑，实际得到 `31 / 8 / 216` 且均
exit 0。上表只采用成功重跑结果；ENOSPC 是已消除的环境中断，不是候选失败。

## 3. Combined compile、format 与 diff

| command | 结果 |
| --- | --- |
| 下列 10-package `cargo check --locked` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

combined check 的 exact package set：

```bash
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
```

命令 exit 0；编译输出只有既有 advisory warnings。

## 4. F419A 三组旧失败闭合

### 4.1 Compiler service conformance

`service_conformance` 已从 F419A 的 harness 构建失败闭合为实际 listing `14`、execution `14/14`。
原阻断测试实际执行通过：

```text
test protocol_identity_tracks_semantics_but_not_diagnostic_text ... ok
```

### 4.2 FileIR execution type representation

FileIR target 实际 listing `2`、execution `2/2`。两个 fixture 在 current Package nominal
语义下已改名并实际执行通过：

```text
test impl_receiver_stays_local_while_contract_parameter_preserves_package_nominal_identity ... ok
test contract_typed_executables_preserve_package_nominal_execution_identity ... ok
```

这闭合 F419A 中旧名称的两项 external-self rewrite 失败。

### 4.3 Runtime combined fixture 八项

`assembly_execution` 实际 `92/92`，supplemental full eval 实际 `216/216`。F419A 的八个失败均在
focused execution 中显示 `ok`：

```text
inline_effect_response_is_materialized_in_spawned_stream_producer_heap
inline_effect_request_finalization_reports_and_clears_unused_setup
package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation
inline_effect_stream_is_consumed_in_buffered_event_order
inline_effect_setup_dispatch_reports_request_subset_mismatch
restricted_service_diagnostic_package_callable_typed_throw_submits_zero
source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable
source_inline_service_effect_sequence_typed_throw_is_caught_then_responds
```

最后一项 typed throw/catch 也单独从实际 execution 日志确认：

```text
test ...source_inline_service_effect_sequence_typed_throw_is_caught_then_responds ... ok
```

## 5. Production suspension/cancellation 终态

在 `artifact-model`、`artifact-identity`、`compiler`、`deployment`、`runtime` Rust code 中反搜：

```text
BoundaryCancellationContract
BoundaryOperationContract.cancellation
BoundaryOperationContract.may_suspend
CallbackContractOperationProjection.may_suspend
```

结果为 0。当前 shape：

- `BoundaryOperationContract` 只保留 parameters、return、stream、callbacks 与
  `effect_guarantee`；
- `CallbackContractOperationProjection` 只保留 contract/local method、slot、ABI、
  executable、receiver ABI、parameters 与 return type。

concrete `may_suspend` owner 保留：

```text
artifact-model/src/executable.rs
  ExecutableSignatureIr.may_suspend
  ExecutableIr.may_suspend

artifact-model/src/effects.rs
  CallableMayEffects.may_suspend

artifact-model/src/package_artifact.rs
  PackageCallableSignature.may_suspend

runtime/linked-program/src/linked.rs
  LinkedExecutable.may_suspend
```

public/link exact equality 保留：

```text
artifact-identity/src/package_artifact/validation/public_instances.rs
  public_signature.may_suspend != method_link.signature.may_suspend

runtime/request/src/http_gateway_target.rs
  linked.may_suspend != signature.may_suspend

runtime/eval/src/runtime_http_gateway.rs
  resolved.executable.may_suspend != callable.signature.may_suspend
```

`skiff-artifact-identity public_instance` 实际 listing `8`、execution `8/8` PASS。

## 6. F415 collection-name mapping 与 F419 initializer

production mapping 链仍可见：

```text
PackageDependency.collection_name_mapping
  -> compiler input-model parse + validate_dependency_collection_name_mapping
  -> driver PackageRequirement exact clone
  -> generated deployment exact clone
  -> linked-program / linker / loader requirement-vs-binding exact equality
  -> active assembly context exact projection
```

关键代码证据包括：

```text
compiler/driver/generated_deployment.rs
  collection_name_mapping: requirement.collection_name_mapping.clone()

runtime/linked-program/src/shared_image.rs
runtime/linker/src/assembly.rs
runtime/loader/src/runtime_assembly/graph_validation.rs
  requirement.collection_name_mapping == binding/provider.collection_name_mapping

runtime/host/src/loader/active_assembly_context.rs
  link.collection_name_mapping.clone()
```

F419 四个 fixture 中 `collection_name_mapping:` initializer 实际计数：

| file | count |
| --- | ---: |
| `ordinary/tests/service_error_consumer.rs` | 4 |
| `ordinary/tests/source_inline_effect_e2e.rs` | 3 |
| `ordinary/tests.rs` | 4 |
| `service_error_channel/tests.rs` | 2 |
| total | **13 = 4 / 3 / 4 / 2** |

## 7. Unified runtime service lane

production 反搜结论：

- `runtime/eval/src/assembly_execution/ordinary.rs` 只定义
  `execute_package_direct`；
- `execute_service_call` 唯一 production 定义位于
  `runtime/eval/src/assembly_execution/async_stream_cancel.rs`；
- `assembly_execution/mod.rs` 的 service dispatch 统一调用
  `async_stream_cancel::execute_service_call`。

没有第二条 ordinary service lane。

## 8. Consumer-visible stream deadline 与 cleanup

三个 consumer-visible deadline probe 在 full runtime eval 的实际 execution 中通过：

```text
test ...provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout ... ok
test ...stream_item_deadline_remains_typed_through_provider_terminal ... ok
test ...terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout ... ok
```

host task/lease cleanup probe 在 `assembly_admission` 的实际 execution 中通过：

```text
test ...typed_execution_service_stream_deadline_releases_provider_task_and_lease ... ok
```

因此 typed timeout 既到达 consumer，也保持 provider terminal 语义，并释放 provider task 与
lease。

## 9. Current positive generation 反搜

canonical positive producer 保持 terminal generation：

```text
artifact-model/src/schema.rs
  PACKAGE_ARTIFACT_SCHEMA_VERSION = "skiff-package-artifact-v9"
  SERVICE_CONTRACT_SCHEMA_VERSION = "skiff-service-contract-v5"

artifact-identity/src/constants.rs
  PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX = "skiff-package-build-v10:sha256"
  PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX = "skiff-package-local-abi-v7:sha256"
  SERVICE_PROTOCOL_IDENTITY_PREFIX = "skiff-service-protocol-v5:sha256"
```

即 Package v9 / Local ABI v7 / build v10 / ServiceContract v5 / protocol v5。真实 legacy rejection
测试中的旧字符串保留为负例，没有计为 current positive producer。

## 10. 判定

required listing、required execution、supplemental full eval、combined check、format/diff、旧 owner
反搜、concrete summary、exact equality、mapping、unified lane、F419A 旧失败闭合以及四个
deadline/cleanup probe 全部 PASS。

**F419G = PASS；F420 解除。**
