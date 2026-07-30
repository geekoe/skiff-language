# P5-F416 Suspension schema and identity current checkpoint result

状态：Complete。

N0 已把 requirement / protocol suspension facts 从 canonical model 与 identity preimage 中原子删除，
同时保留 concrete executable / callable summaries 和 F415 collection mapping。F417 compiler、F418
deployment、F419 runtime 可从本 checkpoint 的 implementation commit 并行派生；下游尚未迁移，
因此本节点不是稳定候选。

## 1. 锚点、提交与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| D93 implementation start | `0517ec481a19b6cac941ca78ea52e276096f96b3` | `8e9bd05aa74a492140227638052bd3764bed02b2` |
| accepted F415 ancestor | `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` | `a2a10789acfc53f190abefcf02447ccdbb598b80` |
| F416 task / implementation parent | `b40514208e31f8cb7a5fcb2ab065d02a554da011` | `a738563dc06e6d7fcfd11a8e905ae7a7763b2886` |
| N0 implementation checkpoint | `7ac42c5e215477a578dc2cca63d1f6d36b248017` | `a1035bfa02fa745368d5bcd6d8ebbc3d9b54722b` |

两次 ancestry gate 均返回成功：

```text
0517ec481a19b6cac941ca78ea52e276096f96b3 is ancestor of implementation HEAD
7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d is ancestor of implementation HEAD
```

implementation commit：

```text
7ac42c5e215477a578dc2cca63d1f6d36b248017
artifact: cut suspension schema generations
```

该提交修改 28 个文件，全部位于 `artifact-model/**` 或 `artifact-identity/**`。没有修改 compiler、
deployment、runtime、Router、scripts、test-runner、cross-system fixture、ecosystem source或设计；
没有 merge、rebase、push，也没有访问 stable/live。

## 2. Schema 与 validation 终态

删除的 requirement / protocol facts：

- `InterfaceMethodSignature.may_suspend`；
- `BoundaryCallbackOperation.may_suspend`；
- `BoundaryOperationContract.may_suspend`；
- `BoundaryOperationContract.cancellation`；
- `BoundaryCancellationContract` enum及其 public re-export。

`InterfaceMethodSignature` 继续只携带 method type params、receiver/parameter/return shape与
`is_native` / `is_provider` / `is_static` flags。没有新增 default effect、alias或替代 suspension 位。

public-instance validation 现在精确执行：

```text
normalized interface shape == normalized implementation shape
normalized interface shape == public callable shape
public PackageCallableSignature.may_suspend
  == exact implementation link ExecutableSignatureIr.may_suspend
```

因此同一 interface requirement 的 concrete `false` / `true` summaries 均可通过，而 public concrete
summary 与 exact implementation link 不等仍 fail closed。WebSocket model 内的 operation gate只继续
验证 unary、callback与公开 shape，不再读取已删除的 provider cancellation/suspension facts。

保留且未 default / 弱化的 concrete facts：

- `ExecutableSignatureIr.may_suspend` / `ExecutableIr.may_suspend`；
- `CallableMayEffects.may_suspend`；
- `CallableSemanticFacts.effects`；
- `BoundaryImplementationRequirements.complete_may_effects`；
- `PackageCallableSignature.may_suspend`；
- `CanonicalPublicCallableSignature.may_suspend`；
- actor、native、receiver builtin concrete summaries。

反向搜索确认 `BoundaryCancellationContract`、`.cancellation` 及三处删除域的 `may_suspend` 在
`artifact-model/**` / `artifact-identity/**` production 中为零；上述 concrete owner仍存在。

## 3. 完整 generation / marker / prefix 表

| domain | implementation start | N0 terminal |
| --- | --- | --- |
| FileIR | schema `skiff-file-ir-v8`；format `skiff-file-ir-format-v6`；prefix `skiff-file-ir-v8:sha256` | 保持 |
| Publication ABI | unit schema `skiff-publication-abi-unit-v1`；prefix `skiff-publication-abi-v1:sha256` | 保持 |
| PackageUnit | `skiff-package-unit-v1` | `skiff-package-unit-v2` |
| legacy Package Local ABI | marker `skiff-package-local-abi-identity-v2`；prefix `skiff-package-local-abi-v2:sha256` | 保持 |
| implementation links | prefix `skiff-package-implementation-links-v1:sha256` | `skiff-package-implementation-links-v2:sha256` |
| legacy Package build | marker `skiff-package-build-identity-v2`；prefix `skiff-package-build-v2:sha256` | marker `skiff-package-build-identity-v3`；prefix `skiff-package-build-v3:sha256` |
| PackageArtifact | `skiff-package-artifact-v8` | `skiff-package-artifact-v9` |
| canonical Package Local ABI | marker `skiff-package-artifact-local-abi-identity-v4`；prefix `skiff-package-local-abi-v6:sha256` | marker `skiff-package-artifact-local-abi-identity-v5`；prefix `skiff-package-local-abi-v7:sha256` |
| canonical Package build | marker `skiff-package-artifact-build-identity-v7`；prefix `skiff-package-build-v9:sha256` | marker `skiff-package-artifact-build-identity-v8`；prefix `skiff-package-build-v10:sha256` |
| PackageSchemaType | marker `skiff-package-schema-type-identity-v1`；prefix `skiff-package-schema-type-v1:sha256` | marker `skiff-package-schema-type-identity-v2`；prefix `skiff-package-schema-type-v2:sha256` |
| PackageSchemaIndex | marker `skiff-package-schema-index-identity-v1`；prefix `skiff-package-schema-index-v1:sha256` | 保持 |
| ContractOperation | marker `skiff-contract-operation-identity-v1`；prefix `skiff-contract-operation-v1:sha256` | 保持 |
| ServiceContractDefinition | `skiff-service-contract-definition-v3` | `skiff-service-contract-definition-v4` |
| ServiceContract | `skiff-service-contract-v4` | `skiff-service-contract-v5` |
| ServiceProtocol | marker `skiff-service-protocol-identity-v4`；prefix `skiff-service-protocol-v4:sha256` | marker `skiff-service-protocol-identity-v5`；prefix `skiff-service-protocol-v5:sha256` |
| ServiceDeploymentInput | `skiff-service-deployment-input-v3` | 保持 |
| ServiceDeployment | schema `skiff-service-deployment-v2`；marker `skiff-deployment-artifact-identity-v2`；prefix `skiff-deployment-artifact-v2:sha256` | 保持 |
| RuntimeAssembly | schema `skiff-runtime-assembly-v2`；marker `skiff-runtime-assembly-identity-v2`；prefix `skiff-runtime-assembly-v2:sha256` | 保持 |
| pointer / path framing | current v1 framing | 保持 |

所有提升项只有一个 current constant / preimage；没有 dual-read、dual-write、default、fallback或旧 hash
复用。PackageSchemaIndex、deployment与assembly的 schema/identity generation不变，嵌套 exact refs会随
新 Package / protocol identity自然改变。

## 4. Old-wire 与 old-generation rejection

| legacy input | owner / evidence | 结果 |
| --- | --- | --- |
| interface method `maySuspend` | strict `InterfaceMethodSignature` serde test | reject |
| callback operation `maySuspend` | strict `BoundaryCallbackOperation` serde test | reject |
| service operation `maySuspend` | strict `BoundaryOperationContract` / `ServiceContract` serde tests | reject |
| service operation `cancellation` | strict `BoundaryOperationContract` / `ServiceContract` serde tests | reject |
| PackageUnit v1 | package identity validator + package resolver | reject |
| implementation-links v1 prefix | current prefix parser `package_implementation_links_identity_hash` | reject |
| legacy Package build v2 prefix | package identity recomputation | reject |
| PackageArtifact v8 | PackageArtifact surface / identity validation | reject |
| canonical Local ABI v6 prefix | declared-vs-computed identity validation | reject |
| canonical build v9 prefix | declared-vs-computed identity validation | reject |
| PackageSchemaType v1 prefix | schema-record identity validation / typed path framing | reject |
| ServiceContractDefinition v3 | strict authoring parser | reject |
| ServiceContract v4 | contract surface validation | reject |
| ServiceProtocol v4 prefix | protocol hash parser / declared-vs-computed validation | reject |

Marker-only preimages are internal typed projections and now emit only terminal markers；exact generation tests锁定
terminal值，旧 marker不再有读取或发射路径。

## 5. Identity mutation 矩阵

| 变化 | PackageCallableId | Package Local ABI | Package build | PackageSchemaType | ServiceContract body / protocol | ContractOperationId |
| --- | --- | --- | --- | --- | --- | --- |
| public concrete `may_suspend: false -> true`，public/link exact同步 | 稳定 | 改变 | 改变 | 不适用 | 不适用 | 不适用 |
| interface requirement相同、concrete false vs true | 同一stable callable path | 两者均合法且各自精确 | 两者均合法且各自精确 | 不变 | 不适用 | 不适用 |
| public concrete summary与implementation link不等 | — | reject | reject | — | — | — |
| callback implementor summary false vs true，callback shape相同 | concrete owner保留 | concrete owner按自身规则变化 | concrete owner按自身规则变化 | 不变 | 不适用 | 不适用 |
| callback parameter / return shape变化 | — | — | — | 改变 | 被operation引用时改变 | 稳定 |
| provider concrete summary false vs true，operation shape相同 | concrete owner保留 | concrete owner按自身规则变化 | concrete owner按自身规则变化 | 不变 | body与protocol均不变 | 稳定 |
| request / response / stream / callback operation shape变化 | — | — | — | reachable schema按shape变化 | protocol改变 | 稳定 |

`service_protocol_mutation_matrix_covers_open_operation_surface` 继续覆盖 request、response、stream与callback
shape；新的 suspension tests显式证明 provider concrete signature不进入ServiceContract canonical body。
deployment / assembly model没有新增 summary wire，schema/identity generation继续保持v2。

## 6. F415 collection mapping 保留

以下 owner与语义完整保留：

```text
PackageRequirement.collection_name_mapping
PackageBinding.collection_name_mapping
RuntimeAssembly package link collection_name_mapping
artifact / deployment / assembly mapping validation
PackageArtifact canonical build packageRequirements preimage
```

两处 DTO仍为 `BTreeMap<String, String>` 且保留
`serde(default, skip_serializing_if = "BTreeMap::is_empty")`。最终测试矩阵为：

| mapping 变化 | Package build | Package Local ABI | validation |
| --- | --- | --- | --- |
| missing vs explicit empty | 相同 | 相同 | accept |
| insertion order变化 | 相同 | 相同 | accept |
| empty -> non-empty | 改变 | 不变 | accept |
| 同source target变化 | 改变 | 不变 | accept |
| empty source / target、显式target collision | — | — | reject |
| unknown source或映射后partial collision | — | — | reject |

本任务没有修改 mapping field、validator或 projection。model full suite中的2个 shared mapping tests和
identity full suite中的 build mutation test均随最终代码通过。

## 7. 验证证据与实际计数

三次 listing 与 required Rust命令均使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际结果 |
| --- | --- |
| `cargo test --locked --manifest-path artifact-model/Cargo.toml --lib -- --list` | 168 tests / 0 benchmarks |
| `cargo test --locked --manifest-path artifact-identity/Cargo.toml --lib -- --list` | 125 tests / 0 benchmarks |
| `cargo test --locked --manifest-path artifact-identity/Cargo.toml --test identity_cli -- --list` | 8 tests / 0 benchmarks |
| `cargo test --locked --manifest-path artifact-model/Cargo.toml --lib` | 168 passed / 0 failed |
| `cargo test --locked --manifest-path artifact-identity/Cargo.toml --lib` | 125 passed / 0 failed |
| `cargo test --locked --manifest-path artifact-identity/Cargo.toml --test identity_cli` | 8 passed / 0 failed |
| `cargo check --locked -p skiff-artifact-model -p skiff-artifact-identity` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

共享 target没有出现 provenance污染，不需要隔离 target，也没有清理共享 cache。未运行 workspace/full
isolated/stable/live。

## 8. 自验收

| 条款 | 代码 / 测试证据 | 结论 |
| --- | --- | --- |
| requirement / callback / service protocol旧facts删除 | model owners、strict serde negatives、zero reverse search | PASS |
| concrete summaries保留 | owner reverse search、Package/publication/actor/native tests | PASS |
| interface false/true均conform | `interface_requirement_accepts_both_concrete_suspension_summaries` | PASS |
| public concrete与exact link mismatch拒绝 | existing public-instance negative matrix | PASS |
| concrete mutation改变Local ABI/build但ID稳定 | public-instance mutation test | PASS |
| callback implementor summary不进入schema identity | `callback_schema_shape_is_identity_bearing_but_implementor_summary_is_not` | PASS |
| provider summary不进入contract/protocol/operation ID | `provider_summary_is_outside_service_contract_protocol_and_operation_identity` | PASS |
| shape mutation仍改变schema/protocol | callback schema + service protocol mutation tests | PASS |
| generation原子切换且旧值拒绝 | exact constants、validators、prefix parsers、stale tests | PASS |
| F415 mapping保留 | field/validator/preimage反搜 + model/identity full suites | PASS |
| ownership与禁止项 | implementation 28 files仅在两个授权root；无stable/live/merge/rebase/push | PASS |

结论：P5-F416 N0 完成，F417 / F418 / F419 可从
`7ac42c5e215477a578dc2cca63d1f6d36b248017` 派生。
