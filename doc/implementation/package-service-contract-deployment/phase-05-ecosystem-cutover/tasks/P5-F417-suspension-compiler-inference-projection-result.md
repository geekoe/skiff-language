# P5-F417 Suspension compiler inference and projection result

状态：Complete（compiler scope；下游 F418 / F419 尚未合流，因此不是 stable 候选）。

compiler 已删除 requirement / protocol 上旧 suspension / cancellation consumer，以 source-internal
`ResolvedCallTarget::InterfaceMethod` 保留已解析动态调用，并把 caller suspension 收敛到唯一 target
矩阵。concrete executable / FileIR / public callable summary、complete effects、provenance 以及 F415
collection mapping 均保持。

## 1. 锚点、提交与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| integrated N0 checkpoint | `c597e3c0e5ecb9d1711b1a25a2660ea9cc972a60` | `715ef42385e58b518e278ef082d78d0ed32b6f79` |
| N0 implementation | `57d0a5551aaa62e5a71655050478c1447f94324d` | `a1035bfa02fa745368d5bcd6d8ebbc3d9b54722b` |
| accepted F415 | `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` | `a2a10789acfc53f190abefcf02447ccdbb598b80` |
| F417 implementation start | `16c17b7d020d90ff5c97ad314f4ceeceaaa363c6` | `fae922e19dbb8ada9ae513b3b0c861c06adf6f2f` |
| F417 implementation | `81d8f2e4722e23e9336c823bc57c639a7ae9bfcd` | `c311e726bc2955f4dbdaa15c3b3dcc99e5636fb1` |

三次 `git merge-base --is-ancestor <anchor> HEAD` 均成功。implementation commit：

```text
81d8f2e4722e23e9336c823bc57c639a7ae9bfcd
compiler: infer suspension from concrete call targets
```

该提交修改 36 个文件，全部位于 `compiler/**`。没有修改 artifact-model、artifact-identity、
deployment、runtime、router、scripts、test-runner、cross-system fixture、ecosystem source或设计；
没有 merge、rebase、push，也没有访问 stable/live。

## 2. Call target 与 caller suspension 终态

| resolved target | caller `may_suspend` |
| --- | --- |
| local function / local impl / actor method | SCC exact；缺失 fact 时 `true` |
| native function / receiver builtin | registry exact；缺失 fact 时 `true` |
| config intrinsic | `false` |
| dependency package callable | `exact_signature.may_suspend`；缺失 signature 时 `true` |
| source-internal interface method | `true` |
| service contract operation | `true` |
| unknown / unresolved | `true` |

新增的 source-internal `InterfaceMethod` target 精确携带：

```text
interface: InterfaceInstantiationRef
method_abi_id: String
slot: u32
```

已解析 interface method 不再降成 `Unknown(UnsupportedDynamicDispatch)`。public
`CallableTargetFact` 没有扩展 wire kind；projection 将该内部 target 显式映射为
`CallableTargetFact::Unknown`，同时内部 interface / method ABI / slot facts不丢失。

dependency transfer 继续从 canonical semantic facts保留 complete effects与 provenance，只用
`exact_signature` 覆盖 caller suspension；缺失 signature fail closed。service
`detached_contract_callee` 不再读取 provider bit，detached shape允许精确保留其余 effect/provenance，
但 caller suspension固定为保守 `true`。test-effect service signature同样只产生保守 caller
summary，不读取或伪造 contract provider bit。

`dependency_exact_signature_controls_lowered_suspend_flag_without_synthetic_calls` 证明
`false` / `true` 只改变 FileIR executable 的 `may_suspend`，两个 body 完全相同且都只有原始一个 call；
没有 synthetic yield、额外 runtime call或“至少挂起一次”语义。

## 3. Interface、concrete handoff 与 wire shape

interface conformance 现在只比较 receiver、参数、返回值与 flags shape。测试中同一 `Runner`
requirement 的 `Immediate.run=false` 与 `Deferred.run=true` 同时 conform。

concrete facts仍逐跳精确：

```text
source callable effects
  -> FileIR ExecutableIr.may_suspend
  -> PackageCallableSignature.may_suspend
  -> Package Local ABI public callable
  == exact implementation executable summary
```

`public_instance_exact_signature_reaches_package_local_abi` 现以 current prelude、对象安全 receiver、
exact package requirement与resolved schema完整运行，证明 FileIR 保留 package symbol、Local ABI
保留 exact PackageSchema，并显式断言 public `may_suspend == implementation executable.may_suspend`。
artifact-identity既有 public-instance negative matrix继续拒绝 public concrete summary与 exact
implementation link不等。

compiler producer / fixture 已删除：

- `InterfaceMethodSignature.may_suspend` 的 copy与initializer；
- `BoundaryCallbackOperation.may_suspend` 的 copy与initializer；
- `BoundaryOperationContract.may_suspend` / `cancellation` 的 producer、reader与fixture；
- `BoundaryCancellationContract` 的 compiler import与诊断分支。

false / true concrete `PackageCallableSignature` 投影为完全相同的 operation contract；JSON同时没有
`maySuspend` 与 `cancellation`。callback schema只保留 parameter / return shape。strict serde
negative确认把任一旧字段加回 operation contract 都会拒绝。反向搜索
`BoundaryCancellationContract`、`.cancellation`、`contract.may_suspend`、
`operation.may_suspend`、`method.may_suspend` 在 `compiler/**` 为零；concrete callable
`may_suspend` owner仍保留。

## 4. Current generation 与 fresh identity

所有 compiler 正向 producer / fixture均使用 terminal current：

| surface | current |
| --- | --- |
| PackageArtifact | v9 |
| canonical Package Local ABI | v7 |
| canonical Package build | v10 |
| PackageSchemaType | v2 |
| ServiceContract | v5 |
| ServiceProtocol | v5 |
| FileIR | v8（保持） |
| Publication ABI | v1（保持） |

一次有界的 std source → lowering → projection probe计算了 fresh current identities，随后在
implementation commit前删除 probe：

```text
build       skiff-package-build-v10:sha256:0dec996a2d6388245539fb000a0284a1561dc21ac3cc6e88ed3fbe0eadfe3d43
local ABI   skiff-package-local-abi-v7:sha256:ce09dc5902ce992d7b362f48ce1ea5466e12fc0e950d4fa90ec99ba46b86db9e
schema idx  skiff-package-schema-index-v1:sha256:593fb4150c7cffbfb1285bc00083abd9b059f2a7a0866e365e37bd9db1cba4bf
Conflict    skiff-package-schema-type-v2:sha256:55e0f59a69a2facc339d89ba12be27a0aaec3e1a60b3211b43259d153b480a4d
db FileIR   skiff-file-ir-v8:sha256:e62485ea5dcd42c0e4552db0e4271bc8bd573ca7478a09bfa238bd2183976cf8
```

这些 exact值已写入 retained positive tests。compiler旧 generation反搜只剩
`stale_package_artifact_schema_and_identity_prefixes_fail_closed` 中故意构造的 v8 / v6 / v9
negative；没有 dual-read、dual-write或兼容 fallback。

## 5. F415 collection mapping 保留

本任务没有修改 mapping owner或validator。链路仍为：

```text
PackageDependency.collection_name_mapping
  -> compiler input-model ingest + shared validation
  -> driver package_requirement exact clone
  -> PackageRequirement.collection_name_mapping
  -> generated deployment exact clone
  -> projection-input exact requirement validation
```

`compiler/input-model/src/dependencies.rs` 继续读取并验证 non-empty mapping；
`compiler/driver/pipeline/mod.rs` 与 `compiler/driver/generated_deployment.rs` 继续调用 `.clone()`，
没有 empty replacement。额外 targeted test
`dependency_mapping_uses_the_frozen_authoring_spelling_and_rejects_collisions` 为 1 passed；driver
existing test `authored_dependency_collection_mapping_reaches_compile_requirement_exactly` 的 source
仍逐跳比较 equality，但在本隔离 checkpoint上不能越过下游 compile blocker执行。

## 6. 验证、实际计数与下游断链

required listing与Rust命令均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

先执行了相同 selector 的 `-- --list`：

| selector | D93 baseline | current listing |
| --- | ---: | ---: |
| core `package_interface` | 5 | 5 |
| source `callable_effects` | 83 | 85 |
| lowering `suspend` | 1 | 2 |
| projection `package_artifact` | 62 | 63 |
| compiled `--lib` | 5 | 6 |
| contract `--lib` | 6 | 7 |
| `service_conformance` | 14 | BLOCKED before harness |
| `file_ir_execution_type_representation` | 2 | BLOCKED before harness |

合理增量分别来自 dependency exact false/true与interface conformance（source +2）、无 synthetic call
FileIR证明（lowering +1）、contract shape独立于concrete summary（projection +1）、internal
interface target到public Unknown mapping（compiled +1）、旧wire strict rejection（contract +1）。
两个被阻断的 integration文件静态仍分别含 14 / 2 个 `#[test]`，本任务未增加或删除测试；由于
Cargo在构建 test harness前失败，不能把静态数量伪报为实际 listing。

| 命令 | 实际结果 |
| --- | --- |
| `cargo test --locked -p skiff-compiler-core package_interface` | 5 passed / 0 failed |
| `cargo test --locked -p skiff-compiler-source callable_effects` | 85 passed / 0 failed |
| `cargo test --locked -p skiff-compiler-lowering suspend` | 2 passed / 0 failed |
| `cargo test --locked -p skiff-compiler-projection package_artifact` | 63 passed / 0 failed |
| `cargo test --locked -p skiff-compiler-compiled --lib` | 6 passed / 0 failed |
| `cargo test --locked -p skiff-compiler-contract --lib` | 7 passed / 0 failed |
| `cargo test --locked -p skiff-compiler --test service_conformance` | BLOCKED before tests |
| `cargo test --locked -p skiff-compiler --test file_ir_execution_type_representation` | BLOCKED before tests |
| `cargo check --locked -p skiff-compiler` | BLOCKED in downstream consumer |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

阻塞均是叶任务已预告的并行下游断链，不是 compiler failure：

```text
runtime/model/src/callback_projection.rs
  reads removed BoundaryCallbackOperation.may_suspend

deployment/src/projection/eligibility.rs
  imports removed BoundaryCancellationContract
  reads removed BoundaryOperationContract.may_suspend
  reads removed BoundaryOperationContract.cancellation
```

上述路径不在 F417 授权 production root，未修改。额外证据：

| targeted test | 结果 |
| --- | --- |
| compiled `public_instance_signature_handoff` | 1 passed / 0 failed |
| input-model collection mapping ingest / validator | 1 passed / 0 failed |
| artifact-identity `public_instance` selector | 8 passed / 0 failed |

没有运行 workspace/full isolated/stable/live；没有清理或改写共享 target cache。

## 7. 自验收

| 条款 | 证据 | 结论 |
| --- | --- | --- |
| interface/callback/contract旧summary consumer删除 | compiler zero reverse search + strict old-wire negative | PASS |
| source-internal interface target明确且public不扩wire | internal field test + public Unknown projection test | PASS |
| caller suspension矩阵唯一 | source transfer + lowering matrix tests | PASS |
| dependency exact false/true与missing fail closed | source 85-test selector + FileIR false/true test | PASS |
| interface/service/unknown保守 true | existing exact target matrix + interface target assertions | PASS |
| conservative true不生成synthetic yield | identical FileIR bodies + one original call | PASS |
| interface concrete false/true均conform | source test + artifact-identity public-instance matrix | PASS |
| public concrete handoff exact、mismatch仍拒绝 | compiled handoff 1/1 + artifact-identity negative matrix | PASS |
| callback/schema/contract wire没有provider bit | projection equality / JSON absence + strict serde negative | PASS |
| complete effects与provenance未弱化 | dependency transfer preserves canonical semantic facts；source full selector通过 | PASS |
| current generations唯一 | positive fixture反搜 + fresh std projection identities | PASS |
| F415 mapping逐跳exact | owner/code反搜 + input-model targeted test | PASS |
| ownership与禁止项 | implementation仅 `compiler/**`；无stable/live/merge/rebase/push | PASS |

结论：P5-F417 compiler checkpoint 完成。F418 / F419 合流后必须重新执行本节三个被下游阻断的
命令及两个 integration selector；本结果不把当前隔离断链误报为 stable failure或stable通过。
