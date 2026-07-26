# P5-F418 Suspension deployment admission result

状态：Complete。

deployment 已迁移到 N0 terminal suspension schema：code-free `ServiceContractRef` 不再与 provider
`may_suspend` summary 比较，已删除 cancellation feature branch；concrete callable facts、完整 effects /
provenance equality、独立 eligibility、exact binding以及 collection mapping仍 fail closed。未提升
deployment / assembly generation，也未新增 summary wire。

## 1. 锚点、提交与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| task branch start | `16c17b7d020d90ff5c97ad314f4ceeceaaa363c6` | `fae922e19dbb8ada9ae513b3b0c861c06adf6f2f` |
| implementation | `f75a3cb10828b02efdf93c3360a4ed2f02f00c44` | `ccd17f09927c6d93329e920ce4b8b09b16f6b790` |

启动 ancestry gate 均成功：

```text
c597e3c0e5ecb9d1711b1a25a2660ea9cc972a60 ancestor=yes
57d0a5551aaa62e5a71655050478c1447f94324d ancestor=yes
7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d ancestor=yes
```

implementation 只修改 6 个 `deployment/**` 文件：

```text
deployment/src/assembly/tests/fixtures.rs
deployment/src/projection/eligibility.rs
deployment/src/projection/tests.rs
deployment/src/projection/tests/eligibility.rs
deployment/src/projection/tests/operation_bindings.rs
deployment/src/storage/tests.rs
```

没有修改 artifact-model、artifact-identity、compiler、runtime、router、scripts、test-runner、
cross-system fixture、ecosystem source或设计；没有 merge、rebase、push，也没有访问 stable/live。

## 2. Admission 保留 / 删除矩阵

| admission 条件 | 终态 | 证据 |
| --- | --- | --- |
| exact operation shape | 保留，mismatch拒绝 | `operations.rs` 的 exact descriptor equality；`unavailable_callable_and_nominal_descriptor_mismatch_fail_closed` |
| exact callable binding / public callable link | 保留，missing / forged / non-public / link drift拒绝 | `validate_public_callable`；`missing_forged_and_implementation_only_callable_ids_fail_closed`、link target negative |
| typed facts与requirements存在 | 保留，缺失拒绝 | `project_operation_bindings` 的 exact map lookup |
| detached / escape / mutation / callback / stream / native eligibility | 保留，unsupported或unsafe拒绝 | `eligibility.rs`；unsafe effects、unknown facts、unsupported stream/callback/native tests |
| unknown effects | 保留，fail closed | `CallableEffectSummary::Unknown` eligibility与facts gate |
| `effects == complete_may_effects` | 保留，mismatch拒绝 | `validate_callable_facts`；`complete may-effects differ` negative |
| provenance equality | 保留，mismatch拒绝 | `validate_callable_facts`；新增 `provenance differs` negative |
| provider effects `may_suspend` 对 contract `may_suspend` | 删除 | N0 contract已无该字段；`validate_effects` 不再做 provider-summary / code-free-contract比较 |
| cancellation unsupported feature branch | 删除 | N0 已删除 cancellation field / enum；deployment反向搜索为零 |

admission 调用顺序没有重排：operation shape / callable binding先验证，随后读取 exact typed facts与
requirements，执行独立 eligibility，最后执行 complete effects与provenance equality。

## 3. 同一 contract 的两个 concrete provider

`code_free_contract_admits_both_provider_summaries_and_exact_refs_change_identity` 构造相同 contract 的
两个 terminal provider。fixture 同步改变所有 concrete owner：

```text
PackageCallableSignature.may_suspend
ExecutableSignatureIr.may_suspend
CallableSemanticFacts.effects.may_suspend
BoundaryImplementationRequirements.complete_may_effects.may_suspend
```

测试证明：

- concrete `false` 与 `true` 均通过 deployment admission；
- 两侧 `ServiceContract`、`service_protocol_identity`、operation descriptor / identity逐字相同；
- 两侧 exact `PackageBuildId` 不同，且 deployment中的 exact implementation ref不同；
- 两侧 `deployment_artifact_identity` 不同；
- 用各自 exact deployment / package ref解析出的 `resolved_packages` 与 `assembly_identity` 不同。

因此 provider summary只影响 concrete Package build以及嵌套 exact ref派生的 deployment / assembly
value，不回流到 ServiceContract、ServiceProtocol或ContractOperation identity。ServiceDeploymentInput
仍为 v3，ServiceDeployment schema / identity仍为 v2，RuntimeAssembly schema / identity仍为 v2。

## 4. F415 mapping preservation

production validator保持未修改：

```text
deployment/src/projection/package_closure.rs
deployment/src/assembly/resolver.rs
```

逐跳证据：

- `exact_package_closure_is_required_and_binding_changes_identity` 使用同一非空
  `collection_name_mapping` 写入 requirement与deployment binding，并继续拒绝 binding drift；
- `collection_mapping_is_preserved_from_requirement_through_assembly_link` 证明同一非空 map从
  `PackageRequirement` 到 `PackageBinding` 再到 assembly package link逐跳 exact相同；
- assembly `package_edges_reject_version_abi_and_build_lookup_mismatches` 保留 mapping drift negative；
- 没有 empty fallback、model default或删除 collision / drift negative。

## 5. 验证证据与实际计数

所有 Cargo listing、test与check均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际结果 |
| --- | --- |
| `cargo test --locked -p skiff-deployment projection -- --list` | 20 tests / 0 benchmarks |
| `cargo test --locked -p skiff-deployment storage -- --list` | 13 tests / 0 benchmarks |
| `cargo test --locked -p skiff-deployment assembly -- --list` | 20 tests / 0 benchmarks |
| `cargo test --locked -p skiff-deployment projection` | 20 passed / 0 failed |
| `cargo test --locked -p skiff-deployment storage` | 13 passed / 0 failed |
| `cargo test --locked -p skiff-deployment assembly` | 20 passed / 0 failed |
| `cargo check --locked -p skiff-deployment` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

D93 listing基线为 `19 / 13 / 20`。当前合理变化为 `20 / 13 / 20`：projection新增一个联合回归，
在同一 test内覆盖 false/true admission、contract / protocol / operation identity稳定以及
deployment / assembly identity分离；storage与assembly selector数量保持不变。

没有运行 workspace/full isolated/stable/live。

## 6. Combined-tree 待验证项

本 branch 与 F417 / F419 从同一 N0 checkpoint并行，未包含 compiler / runtime consumer提交。主 Agent
合流后仍需在 combined tree：

1. 用 F417 fresh compiler产出的 terminal PackageArtifact v9、Local ABI v7、build v10 fixture复验同一
   code-free contract的 concrete `false` / `true` provider均进入本 admission；
2. 复验 fresh artifact的 exact build refs继续令 deployment / RuntimeAssembly v2 identity分离；
3. 运行阶段 combined proof，确认 compiler、deployment、runtime之间没有旧 suspension /
   cancellation wire残留，并保留 F415 non-empty mapping逐跳 exact。

上述是并行合流证据，不是本 implementation 的未完成项。
