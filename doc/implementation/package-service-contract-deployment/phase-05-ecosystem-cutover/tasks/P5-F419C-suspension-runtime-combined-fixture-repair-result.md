# P5-F419C Suspension runtime combined fixture repair result

状态：**TASK_SCOPE_EXPANDED**（合同内三项机械适配已形成安全 checkpoint；同一 8 个
runtime-eval fixture 的原首错已消失，但执行继续暴露三类未授权的 current-authoring 缺口）。

## 1. Exact checkpoint 与边界

| 项目 | commit | tree |
| --- | --- | --- |
| task start | `43c82f039e11f182a1e8d315fe85dcde60d0408c` | `692551aca0b65c5062a620fb3d9de0414c771fbf` |
| 合同内 test-only checkpoint | `b4472e0bae827dfefdbd7cc53ca21db37ece6b7b` | `dd4f0f969fb63a18d42b44395ff111d26f818d1f` |

启动时下列 ancestry gate 均为 exit `0`：

```text
087469235de2d1bb67965bce884b963d537c3f47
2b9d29eea9a65ab323240f1e6c34b3e3b29c7403
7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d
```

checkpoint 只修改任务授权的两个 runtime test 文件，没有修改 production、validator、
artifact model、compiler、deployment、tooling、设计或其它 fixture。没有派子 Agent，没有
merge/rebase/push，没有访问 stable/live、instance 或 watch registry。

## 2. 已完成且独立有效的三项机械适配

1. `package_direct_fixture_with_caller` 的单一 `PackageCallableId` 从旧的
   `callable:package-direct-mutate` 改为：

   ```text
   pkg-callable:example.package-direct-callee:mutate
   ```

   现有 fixture 继续把同一变量传给 PackageArtifact public symbol、semantic facts、
   callable link、caller external ref 和 package-direct call target；没有放宽 canonical
   callable-id validation。
2. `write_consumer_package` 的 `api.yml` 从零字节改为 canonical empty map `{}\n`。
3. `write_std_effect_consumer_package` 的 `api.yml` 从零字节改为 canonical empty map `{}\n`。

三项改动均已推进首错：

- 六个 package-direct fixture 不再报
  `public callable mutate has non-canonical callable id`；
- 两个 source-inline fixture 不再报 `api.yml must not be empty`。

测试的 expected effects、same-heap、typed throw、stream 顺序、request subset 和 diagnostic
断言均未修改。

## 3. Focused listing 与 execution

所有 Cargo 命令使用共享 target：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 结果 |
| --- | --- |
| `cargo test --locked -p skiff-runtime-eval assembly_execution -- --list` | PASS；实际 `92 tests / 0 benchmarks` |
| `cargo test --locked -p skiff-runtime-eval assembly_execution` | **FAIL**；实际 `84 passed / 8 failed / 124 filtered out` |
| `cargo check --locked -p skiff-runtime-eval` | PASS；只有既有 advisory warnings |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

发现 scope expansion 后按工作流停止；没有继续运行 full-eval listing/execution，因此不能声明预期的
`216` 项或 8 个 fixture 闭合。

## 4. 推进后的三类新首错

### 4.1 六个 package-direct fixture：缺 canonical implementation function export

以下六项均已越过 callable-id validation：

```text
inline_effect_response_is_materialized_in_spawned_stream_producer_heap
inline_effect_request_finalization_reports_and_clears_unused_setup
package_direct_same_heap_uses_canonical_executor_and_exposes_callee_mutation
inline_effect_stream_is_consumed_in_buffered_event_order
inline_effect_setup_dispatch_reports_request_subset_mismatch
restricted_service_diagnostic_package_callable_typed_throw_submits_zero
```

共同新首错位于
`runtime/eval/src/assembly_execution/ordinary/tests.rs::package_direct_fixture_with_caller`
为 callee 分配 package identities 时：

```text
InvalidPackageArtifact {
  message: "public function callable targets must exactly match implementation function links;
            expected {}, got {(<callee FileIR identity>,
            \"package_direct.callee\", 0)}"
}
```

fixture 的 `callable_package` / `stream_callable_package` 写了 public symbol、callable link、
semantic facts 与 boundary projection，但由 `private_package` 继承的
`implementation_links.functions` 仍为空。补写 current canonical `mutate` function export
属于新的 fixture-authoring hunk，不在 F419C 已列出的三项机械适配内，因此本节点没有吞入。

### 4.2 typed-throw source fixture：旧 contract requirement 不是 exact reachable closure

`source_inline_service_effect_sequence_typed_throw_is_caught_then_responds` 已越过 API control-file
解析，随后在 consumer source package compile 失败：

```text
Compile(ContractValidation {
  message: "contract dependency validation failed: ServiceContract dependency `payments`
            package schema requirements are not the exact operation-reachable closure"
})
```

精确 owner 是同一授权文件
`source_inline_effect_e2e.rs::publish_open_error_service_contract`：`echo` 的 parameter/return
均为 builtin `string`，但 fixture 仍把 `errors.Failure` 写入
`package_type_requirements`。typed throw payload 由 consumer 的直接 `errors` package dependency
提供；修正 open-error contract 的 exact operation-reachable requirement 不需要 production 或
validator 修改，但这是本轮首错之后新暴露的 fixture authoring 迁移。

### 4.3 std source fixture：dependency public schema 未真实 hydrate

`source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable` 已越过 API
control-file 解析、compile、overlay lowering 和 exact std callable/registration 断言，随后在
link fixture 失败：

```text
runtime/eval/src/test_support.rs:
package fixture with public schema records must hydrate its real index and records
```

精确 owner 是
`source_inline_effect_e2e.rs::{execute_overlay_case,hydrate_packages}`：它读取了 dependency FileIR，
但仍把带 public schema records 的 canonical std package 交给只接受 empty-schema fixture 的
`crate::test_support::link_package_fixture`。该 source fixture 应使用已有
`HydratedPackageCode` 路径，为 overlay 使用其 compiler-emitted schema，并从
`CanonicalArtifactStore` 解析 dependency 的真实 index/records；不需要修改共享 test support 或
runtime production。

## 5. 最小 successor 任务

建议建立一个独立、仍只写下列两个 test 文件和自身 result 的 successor：

```text
runtime/eval/src/assembly_execution/ordinary/tests.rs
runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs
```

授权内容应精确为：

1. 为 unary/stream callee 的 public path `mutate` 补同一 executable index、symbol 和 exact
   signature 的 `implementation_links.functions` export，保持 canonical callable id、
   callable link target 和 caller ref 逐值一致；
2. 把 open-error `echo` contract 的 package type requirements 收敛为其 builtin-only
   operation 的 exact reachable closure，同时保留 consumer 对 `errors.Failure` 的直接 package
   dependency、typed catch/throw 和 ordered response 语义；
3. 让 `execute_overlay_case` 使用真实 `HydratedPackageCode`：overlay 使用 compiler-emitted
   schema，canonical dependencies 使用 store-resolved index/records；不得放宽
   `test_support` 的 fail-closed assertion。

successor 应先逐项执行上述 8 个 exact test，再按原 F419C 顺序重跑：

```text
focused listing/execution：92
full eval listing/execution：216
cargo check、cargo fmt --check、git diff --check
```

只有 focused `92/92`、full eval `216/216` 且同一 8 项全部闭合后，才能恢复 F419C complete
结论并进入新的 combined gate。
