# P5-F419A Suspension consumer combined gate result

状态：**FAIL**（只读 combined code gate；F420 不解除）。

三个 consumer 的 production 静态终态、combined compile、format 与 diff 均通过，但 required test
matrix 没有全绿：

- 21 个 required listing 中 20 个成功，`service_conformance` 在 harness 生成前编译失败；
- 21 个 required execution 中 17 个命令成功、4 个命令失败；
- 失败包括一个旧 contract suspension test access、两个 FileIR integration fixture，以及
  runtime-eval 中同一组 8 个 integration fixture（focused selector 与 supplemental full eval
  各执行并各失败一次）。

本节点没有修复这些失败。以下计数全部来自实际 `-- --list` 或实际 test execution；没有用
`#[test]` 静态数量代替 listing。

## 1. 精确候选、启动门禁与只读边界

| 锚点 | commit | tree / 结果 |
| --- | --- | --- |
| combined production candidate | `2b9d29eea9a65ab323240f1e6c34b3e3b29c7403` | `fc6e7bfb05f4011eb4e0337944507ca3bc67d0cd` |
| accepted F415 | `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` | ancestor PASS |
| gate start HEAD | `510d099ab281332ef458208534112912625b0f1b` | `b38f954e2a2e7f101c849698ec1731cbe6f94f84` |

启动证据：

```text
git merge-base --is-ancestor 2b9d29eea9a65ab323240f1e6c34b3e3b29c7403 HEAD
exit 0

git rev-parse 2b9d29eea9a65ab323240f1e6c34b3e3b29c7403^{tree}
fc6e7bfb05f4011eb4e0337944507ca3bc67d0cd

git merge-base --is-ancestor 7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d HEAD
exit 0
```

gate start HEAD 只比 candidate 多本任务 task 文档：

```text
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/P5-F419A-suspension-consumer-combined-gate.md
```

因此 production / test 与 exact candidate 相同。启动时 worktree clean。所有 Cargo listing、test
与 combined check 均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

没有修改 production / test 或 fixture；shared target cache 只承载上述 Cargo 命令的正常增量输出，
没有清理或手工改写。没有 merge、rebase、cherry-pick、push、stable/live、instance 或子 Agent
操作。

首个 listing 的 Cargo 子命令完成后，外层 zsh 汇总函数因使用其只读变量名 `status` 而退出；完整
21-selector listing batch 随即在 bash 下从头重跑。下表只采用完整重跑的权威结果，这不是代码
门禁失败。

## 2. Required listing 与 execution 矩阵

所有 listing 命令均为表中 test command 后追加 `-- --list`。listing 数量是实际输出中
`: test` / `: benchmark` 项的计数。

| test command | listing | execution |
| --- | --- | --- |
| `cargo test --locked -p skiff-compiler-core package_interface` | PASS，5 tests / 0 benchmarks | PASS，5 passed / 0 failed |
| `cargo test --locked -p skiff-compiler-source callable_effects` | PASS，85 / 0 | PASS，85 / 0 |
| `cargo test --locked -p skiff-compiler-lowering suspend` | PASS，2 / 0 | PASS，2 / 0 |
| `cargo test --locked -p skiff-compiler-projection package_artifact` | PASS，63 / 0 | PASS，63 / 0 |
| `cargo test --locked -p skiff-compiler-compiled --lib` | PASS，6 / 0 | PASS，6 / 0 |
| `cargo test --locked -p skiff-compiler-contract --lib` | PASS，7 / 0 | PASS，7 / 0 |
| `cargo test --locked -p skiff-compiler --test service_conformance` | **FAIL**，exit 101 before harness；无实际 listing | **FAIL**，exit 101 before tests；0 executed |
| `cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation` | PASS，2 / 0 | **FAIL**，0 passed / 2 failed |
| `cargo test --locked -p skiff-deployment projection` | PASS，20 / 0 | PASS，20 / 0 |
| `cargo test --locked -p skiff-deployment storage` | PASS，13 / 0 | PASS，13 / 0 |
| `cargo test --locked -p skiff-deployment assembly` | PASS，20 / 0 | PASS，20 / 0 |
| `cargo test --locked -p skiff-runtime-capability-context execution_control` | PASS，1 / 0 | PASS，1 / 0 |
| `cargo test --locked -p skiff-runtime-request execution_budget` | PASS，6 / 0 | PASS，6 / 0 |
| `cargo test --locked -p skiff-runtime-model callback_projection` | PASS，3 / 0 | PASS，3 / 0 |
| `cargo test --locked -p skiff-runtime-eval assembly_execution` | PASS，92 / 0 | **FAIL**，84 passed / 8 failed |
| `cargo test --locked -p skiff-runtime-native callback_adapter` | PASS，7 / 0 | PASS，7 / 0 |
| `cargo test --locked -p skiff-runtime-linker assembly` | PASS，30 / 0 | PASS，30 / 0 |
| `cargo test --locked -p skiff-runtime-loader runtime_assembly` | PASS，17 / 0 | PASS，17 / 0 |
| `cargo test --locked -p skiff-runtime-host assembly_admission` | PASS，31 / 0 | PASS，31 / 0 |
| `cargo test --locked -p skiff-artifact-identity public_instance` | PASS，8 / 0 | PASS，8 / 0 |
| `cargo test --locked -p skiff-runtime-eval --lib` | PASS，216 / 0 | **FAIL**，208 passed / 8 failed |

命令级汇总：

```text
listing commands   20 PASS / 1 FAIL
execution commands 17 PASS / 4 FAIL
successful listing item occurrences 634
execution occurrences 616 passed / 18 failed
```

`assembly_execution` 的 8 个失败也包含在 full eval 的同一 8 个失败中；因此上面的 18 是按命令
执行次数计算的 occurrence，不是 18 个不同 test。

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

编译输出含既有 advisory warnings，但 exit 0。

## 4. Blocking failures、首错与 owner

### 4.1 Compiler service conformance test 仍访问已删除 provider bit

listing 与 execution 均在 test harness 生成前失败。首错：

```text
compiler/tests/service_conformance.rs:395:49
error[E0609]: no field `may_suspend` on type `&mut BoundaryOperationContract`
```

该 test 的 `protocol_identity_tracks_semantics_but_not_diagnostic_text` 仍执行：

```text
changed.operations.get_mut("echo").unwrap().may_suspend = true
```

production contract 已正确删除该字段；残留位于 compiler integration test。最小 owner：
F417 / `compiler/tests/service_conformance.rs`。该失败阻断 F420。

### 4.2 FileIR execution type representation 两个 fixture 均失败

实际 listing 为 2；execution 为 0 passed / 2 failed。两个 test：

```text
impl_receiver_and_contract_parameter_keep_distinct_execution_roles
contract_typed_executables_have_one_opaque_execution_representation
```

共同首错：

```text
Compile(ContractValidation {
  message: "package example.com/file-ir-execution-types@1.0.0 File IR module main
            packageSymbols contains unrewritten external self reference through
            package id example.com/file-ir-execution-types"
})
```

最小 owner：F417 compiler integration fixture / FileIR package-symbol rewrite handoff，
`compiler/tests/file_ir_execution_type_representation.rs`。该失败阻断 F420。

### 4.3 Runtime eval 的 8 个 combined fixture mismatch

focused `assembly_execution` 实际执行 84 passed / 8 failed；supplemental full eval 实际执行
208 passed / 8 failed。两者是同一组 8 个失败。

六个 ordinary/package-direct fixture：

```text
inline_effect_response_is_materialized_in_spawned_stream_producer_heap
inline_effect_request_finalization_reports_and_clears_unused_setup
package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation
inline_effect_stream_is_consumed_in_buffered_event_order
inline_effect_setup_dispatch_reports_request_subset_mismatch
restricted_service_diagnostic_package_callable_typed_throw_submits_zero
```

共同首错：

```text
InvalidPackageArtifact {
  message: "public callable mutate has non-canonical callable id
            callable:package-direct-mutate,
            expected pkg-callable:example.package-direct-callee:mutate"
}
```

另两个 source-inline fixture：

```text
source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable
source_inline_service_effect_sequence_typed_throw_is_caught_then_responds
```

共同首错：

```text
PackageConfig(ParsePackageManifest {
  path: ".../consumer/api.yml",
  message: "api.yml must not be empty"
})
```

最小 owner：F419 `runtime/eval` combined test fixtures
（`assembly_execution/ordinary/tests.rs` 与
`assembly_execution/ordinary/tests/source_inline_effect_e2e.rs`）。这些是父结果已预告但未在真实
combined tree 闭合的 N1/N3 integration mismatch；均阻断 F420。

## 5. Production suspension/cancellation 反搜

production 结论为 PASS：

- `BoundaryCancellationContract` 在 `artifact-model`、`artifact-identity`、`compiler`、
  `deployment`、`runtime` Rust production/code 搜索为 0；
- `BoundaryOperationContract` 当前字段只有 parameters、return、stream、callbacks 与
  `effect_guarantee`，没有 `may_suspend` 或 `cancellation`；
- `CallbackContractOperationProjection` 当前字段只有 contract/local method、slot、ABI、
  executable、receiver ABI、parameters 与 return type，没有 `may_suspend`；
- runtime/deployment/compiler production 不再访问上述旧字段。

`compiler/tests/service_conformance.rs:395` 的 test-only 残留没有被冒充 production 残留，但它真实
导致 required test target 无法编译，因此已在 4.1 单独判 FAIL。

## 6. Concrete summary 与 exact equality 保留

以下 concrete owners 仍在：

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

public/link exact equality仍在：

```text
artifact-identity/src/package_artifact/validation/public_instances.rs:588
  public_signature.may_suspend != method_link.signature.may_suspend

runtime/request/src/http_gateway_target.rs:359
  linked.may_suspend != signature.may_suspend

runtime/eval/src/runtime_http_gateway.rs:340
  resolved.executable.may_suspend != callable.signature.may_suspend
```

`skiff-artifact-identity public_instance` 实际 listing 8，execution 8/8 PASS。

## 7. F415 mapping 链与 F419 initializer

accepted F415 到 candidate 的下列 production owner diff 均为空：

```text
runtime/linked-program/src/shared_image.rs
runtime/linker/src/assembly.rs
runtime/loader/src/runtime_assembly/graph_validation.rs
runtime/host/src/loader/active_assembly_context.rs

compiler/input-model/src/dependencies.rs
compiler/driver/pipeline/mod.rs
compiler/driver/generated_deployment.rs
```

production 链仍可见：

```text
PackageDependency.collection_name_mapping
  -> compiler input-model parse + validate_dependency_collection_name_mapping
  -> driver PackageRequirement exact clone
  -> generated deployment exact clone
  -> linked-program / linker / loader requirement-vs-binding exact equality
  -> active assembly context exact projection
```

F419 四个 fixture 文件中 current initializer 实际计数：

| file | count |
| --- | ---: |
| `ordinary/tests/service_error_consumer.rs` | 4 |
| `ordinary/tests/source_inline_effect_e2e.rs` | 3 |
| `ordinary/tests.rs` | 4 |
| `service_error_channel/tests.rs` | 2 |
| total | **13 = 4 / 3 / 4 / 2** |

dynamic dependency edge 仍用 `requirement.collection_name_mapping.clone()`；其余无 mapping fixture
显式用 `BTreeMap::new()`。

## 8. Unified runtime lane

production 反搜结论为 PASS：

- `runtime/eval/src/assembly_execution/ordinary.rs` 只有
  `execute_package_direct`，无 service executor、service validation 或 service lane；
- production `execute_service_call` 唯一定义位于
  `runtime/eval/src/assembly_execution/async_stream_cancel.rs`；
- `dispatch_in_process_boundary` 对 Unary / ServerStream 统一调用
  `async_stream_cancel::execute_service_call`，Unsupported stream fail closed；
- 另一个直接调用只位于 `#[cfg(test)] service_error_convergence.rs`，不是第二个 production lane。

## 9. Consumer-visible stream deadline 证据

required eval listing 实际列出 8 个 deadline tests，其中包括：

```text
provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout
stream_item_deadline_remains_typed_through_provider_terminal
terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout
stream_terminal_item_and_publication_deadlines_remain_typed
```

required host listing实际列出：

```text
typed_execution_service_stream_deadline_releases_provider_task_and_lease
```

为了把 broad eval 的 8 个无关 ordinary fixture failure 与 deadline 终态分开，额外逐条执行了三条
真实 consumer probe 和一条 concrete host cleanup probe：

| exact probe | execution |
| --- | --- |
| `provider_stream_deadline_terminal_reaches_pending_consumer_as_typed_timeout` | 1 passed / 0 failed |
| `stream_item_deadline_remains_typed_through_provider_terminal` | 1 / 0 |
| `terminal_publication_deadline_replaces_blocked_terminal_with_typed_timeout` | 1 / 0 |
| host `typed_execution_service_stream_deadline_releases_provider_task_and_lease` | 1 / 0 |

前三条通过 `TestStreamRuntime`-backed `StreamRuntime::next` 让 pending consumer 实际观察
`ExecutionBudgetExceeded { reason: DeadlineExceeded }`，不是只检查 helper enum。host probe 还实际
观察 detached provider task 启动后归零、provider request stream lease 释放，以及 request-scope
teardown 后 concrete stream registry 归零。

## 10. Current generation 与 predecessor 反搜

current positive producer constants为：

| surface | current |
| --- | --- |
| PackageArtifact schema | `skiff-package-artifact-v9` |
| Package Local ABI identity | `skiff-package-local-abi-v7:sha256` |
| Package artifact build identity | `skiff-package-build-v10:sha256` |
| ServiceContract schema | `skiff-service-contract-v5` |
| ServiceProtocol identity | `skiff-service-protocol-v5:sha256` |

producer evidence：

```text
compiler/projection/src/package_artifact/projection.rs
  emits PACKAGE_ARTIFACT_SCHEMA_VERSION

artifact-identity/src/package_artifact/projection.rs
  derives with PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_PREFIX
  derives with PACKAGE_ARTIFACT_BUILD_IDENTITY_PREFIX

artifact-identity/src/contract.rs
  derives with SERVICE_PROTOCOL_IDENTITY_PREFIX
```

terminal predecessor tokens `package-artifact-v8`、`local-abi-v6`、`package-build-v9` 与
`service-protocol-v4` 的 relevant matches 只在 `#[cfg(test)]` stale-generation rejection
或 compiler negative fixture 中；它们没有成为 positive producer、dual read/write 或 fallback。
其他对象家族的独立版本不被误判为本 suspension generation 的残留。

## 11. 判定

| gate | 结论 |
| --- | --- |
| exact candidate/tree/ancestry | PASS |
| required listings | **FAIL**（20/21 commands） |
| required executions | **FAIL**（17/21 commands） |
| supplemental full eval | **FAIL**（208/216 tests） |
| combined check | PASS |
| format / diff | PASS |
| production old-owner reverse search | PASS |
| concrete summary / exact equality | PASS |
| F415 mapping + F419 `4/3/4/2` | PASS |
| unified lane | PASS |
| consumer-visible stream deadline probes | PASS |
| current positive generation | PASS |
| overall | **FAIL** |

P5-F419A 不解除 F420。恢复条件是由对应 compiler test owner 与 runtime/eval fixture owner 在后续授权
任务中修复后，对新的 exact candidate 重新执行本 combined gate；本只读节点没有进行任何修复。
