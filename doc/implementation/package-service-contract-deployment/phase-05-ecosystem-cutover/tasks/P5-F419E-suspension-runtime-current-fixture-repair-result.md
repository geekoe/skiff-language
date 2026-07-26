# P5-F419E Suspension runtime current fixture repair result

状态：**TASK_SCOPE_EXPANDED**（三个授权的 current-fixture authoring 数据流已经形成
test-only 安全 checkpoint；7/8 exact tests 已闭合。typed-throw source fixture 随后暴露
`service_error_channel` production 对 compiler-current public/implementation 同址 type export
的未授权歧义判断）。

## 1. Exact checkpoint 与边界

| 项目 | commit | tree |
| --- | --- | --- |
| task start | `9efb9785deda3c170f0bc674a4c31e4ac0d18585` | `6036f8efda3e990492ff39d6bd733b039113eff8` |
| test-only checkpoint | `66678822c810d7675ce84e25f2da9613238c15f3` | `2f95899f91418a0bcd7cd258f1e30512dad51ab6` |

启动时下列 ancestry gate 均为 exit `0`：

```text
b7f7530d4b28b5c84e849a0ea2358c02ed435193
332c98d588311f0b260ff3213f8b5488f103c193
7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d
```

checkpoint 只修改任务授权的两个 runtime test 文件，没有修改 production、shared test
support、validator、artifact model、compiler、deployment、tooling 或设计。没有派子 Agent，
没有 merge/rebase/push，没有访问 stable/live、instance 或 watch registry。

## 2. 三个 authoring 数据流

### 2.1 Package-direct canonical function export

unary 与 stream callee 都从 executable `#0` 直接构造 `ExecutableExport`，因此
`implementation_links.functions["mutate"]` 与 callable link 共用同一 FileIR identity、
module、executable index、symbol 和 executable-derived exact signature。public path 仍为
`mutate`，callable id 仍为：

```text
pkg-callable:example.package-direct-callee:mutate
```

public symbol、semantic facts、boundary projection、callable link、caller external ref 与
package-direct target 仍复用同一个 callable id；没有放宽 canonical id、public-function target
或 link validation。六个 exact tests 均已越过 F419C 的 empty implementation export 首错并
保持 same-heap、effect consumption/finalization、request subset、stream order 与 diagnostic
断言。

### 2.2 Typed-throw source contract closure

`echo(string) -> string` 的 `package_type_requirements` 现在是 builtin-only operation 的 exact
empty closure。fixture 仍解析并确认 canonical error package 的 public `Failure` schema，
consumer manifest 仍保留 direct `errors` package dependency，source/test 仍保留：

- `errors.Failure` typed test-double throw；
- `catch<errors.Failure>`；
- first throw 后的 ordered second response。

compiler-current payload ref 是 direct dependency
`PackageRefIr::Dependency { dependency_ref: "errors" }`，checkpoint 对其 public path 和 exact
local ABI expectation 做了强断言，不再误断言为 `PackageId` ref。执行使用非空 request trace，
以保留 request-local typed exception 的 canonical correlation。

该 exact test 已依次越过 contract compile、overlay lowering、direct dependency/ABI assertion、
真实 schema hydration、link 和 request-local exception construction，但随后命中第 4 节的
production blocker，因而不能声明 typed catch 与 ordered second response 已闭合。

### 2.3 Source overlay 与 canonical dependency hydration

`execute_overlay_case` / `hydrate_packages` 现在构造真实 `HydratedPackageCode`：

- overlay 使用 compiler-emitted FileIR、`package_schema_index` 与
  `package_schema_type_records`；
- 每个 canonical dependency 的 FileIR 从 `CanonicalArtifactStore` 读取，schema index/records
  由 `resolve_package_artifact_schema` 解析；
- fully hydrated inputs 直接交给 canonical runtime-assembly linker。

没有伪造 empty schema，也没有修改或放宽
`crate::test_support::link_package_fixture` 对 public schema fixture 的 fail-closed assertion。
真实 std schema 的执行使用本文件既有测试惯例的 16 MiB scoped runtime thread，测试语义与
production/runtime 请求不变；std exact test 已通过。

## 3. 八项 exact matrix

| exact test | 结果 |
| --- | --- |
| `package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation` | PASS |
| `inline_effect_setup_dispatch_reports_request_subset_mismatch` | PASS |
| `inline_effect_request_finalization_reports_and_clears_unused_setup` | PASS |
| `restricted_service_diagnostic_package_callable_typed_throw_submits_zero` | PASS |
| `inline_effect_stream_is_consumed_in_buffered_event_order` | PASS |
| `inline_effect_response_is_materialized_in_spawned_stream_producer_heap` | PASS |
| `source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable` | PASS |
| `source_inline_service_effect_sequence_typed_throw_is_caught_then_responds` | **BLOCKED**；命中第 4 节 production owner |

实际 exact 计数为 `7 passed / 1 blocked`。

## 4. 新 production blocker

compiler-current error PackageArtifact 合法地为同一 FileIR type `#0` 生成两个精确用途不同的
implementation type links：

```text
Failure      -> public API path
main.Failure -> implementation source path
```

真实 schema hydration 后，typed throw 到达
`runtime/eval/src/assembly_execution/service_error_channel.rs::public_artifact_identity_for_addr`。
该 production 函数只按 FileIR identity 与 type index 收集
`implementation_links.types`，并要求结果恰好一个，因此把上述 public/implementation 同址
export 拒绝为：

```text
InvalidArtifact("service-error execution address has ambiguous implementation type exports")
```

精确 owner 在 `service_error_channel.rs` 的 public schema identity lookup/validation，而不在本任务
授权的 fixture、schema hydration 或 shared test support。修复需要 production 授权，并应以
schema index 的 exact public path 区分 public export，同时继续 fail closed；本节点没有猜测或
实现该 production 语义。

## 5. Focused 与静态验证

所有 Cargo 命令使用共享 target：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p skiff-runtime-eval assembly_execution -- --list` | PASS；`92 tests / 0 benchmarks` |
| `cargo test --locked -p skiff-runtime-eval assembly_execution` | **FAIL**；`91 passed / 1 failed / 124 filtered out`，唯一失败为 typed-throw production blocker |
| `cargo check --locked -p skiff-runtime-eval` | PASS；只有既有 advisory warnings |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

发现未授权 production owner 后按任务规则停止；没有运行 full-eval listing/execution，因此不能
声明预期 `216/216`。在 production blocker 获得独立授权并修复前，本 checkpoint 不应作为
F419E complete 或 combined gate 通过的证据。
