# P5-D93 Suspension current-base reconciliation audit result

状态：Complete；`TASK_EXECUTABLE`。

F395 冻结的 inferred-suspension 语义在 current code 上没有出现设计冲突。实现必须从包含
F415 accepted commit 的 Skiff integration descendant 启动；当前最小 gate 是：

```text
commit  7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d
tree    a2a10789acfc53f190abefcf02447ccdbb598b80
```

本审计分支只新增本 result。它自身基于 pre-F415 task branch，不是 implementation start；主 Agent
必须先把 result 带回上述 integration descendant，再派生实现任务。

## 1. 锚点、父节点与审计边界

| 锚点 | commit | tree | 用途 |
| --- | --- | --- | --- |
| D93 current production anchor | `91e5475d18af9b30adcc01dc4ea2ba41e3d1e10b` | `186a877f8259f274cd79a7588fc204c1e2bec467` | current owner / generation 起点 |
| D93 task worktree start | `feaab16912c06bea7617c2f3ef5500b750a94075` | `6be84b8ef48355961b6dcaac15f8f1689da47ab9` | 只含 D93 task definition |
| accepted post-F415 integration | `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` | `a2a10789acfc53f190abefcf02447ccdbb598b80` | 唯一 implementation start gate |
| accepted F415 production payload | `018e6f8758124a1cfaa6d376f4e7752a7059142d` | `cc2a8af1644b0779d3133f9c04d097ae1b1bc095` | integration 上的 mapping implementation |
| Internals current integration | `960cc4bd722cbbad41fdd5e064663ad505e4f3ac` | `33a838176990193cd01be495a7b692623baa4793` | Relay canonical source与 receipt |

F415 result 记录的 agent implementation 是
`25fa06ed5baa8c56d829abae699dbf175146501f` /
`02cac60583ef001cb80d5199c5a83c75cfb81b48`；它不是 accepted integration ancestor。当前验收以重放后的
`018e6f87` production payload及完整 `7303af9b` gate 为准，不以孤立 agent commit 为启动点。

直接父节点已完整重读：

- `P5-F395-inferred-suspension-implementation-audit-result.md`；
- repository 中实际存在的
  `P5-F409-service-manifest-typed-contract-driver-result.md`；
- `P5-F413-relay-service-calls-and-http-checkpoint-migration-result.md`；
- `P5-F415-collection-mapping-current-integration.md` 与 accepted
  `P5-F415-collection-mapping-current-integration-result.md`。

D93 task 中写的
`P5-F409-typed-service-selection-contract-driver-result.md` 不存在；这是父节点文件名漂移，不是语义
blocker。还沿引用重读了 F391、F382，以及 commit
`018818...` 中的 F393 result。

审计只执行 read/search、test listing、isolated Relay authoring diagnostic和静态测试。没有修改
production/test/design，没有修改 F415 worktree，没有 merge、rebase、push，也没有访问 stable
instance、live store、MongoDB或外部服务。Relay diagnostic 只使用脚本拥有并清理的 isolated
temporary ecosystem store。

## 2. Current-base 结论

1. `91e5475d..7303af9b` 没有修改 suspension 字段 owner、generation constants或 schema constants。
2. 两个锚点上的 suspension 反搜集合完全相同；F415 没有暗中修复或改变 F395 语义。
3. serviceCalls 已占用 PackageArtifact v8、canonical build prefix v9、build preimage marker v7，
   ServiceDeploymentInput 已占用 v3；终态必须在这些 current generations 之后继续单调切代。
4. F415 新增的 `collection_name_mapping` 必须逐跳保留。它改变相关 identity value但不提升 schema
   generation；suspension 迁移不得借 fixture adaptation 丢字段、补 fallback或清空真实 mapping。
5. F395 的旧“service-only source-base skew” G0 已关闭。current Relay 是 canonical
   `package.yml + service.yml + api.yml` authoring，能够越过 manifest/parser/selection，精确停在
   interface requirement与 concrete implementation suspension 比较。
6. accepted F415 tree 新暴露 13 个 runtime/eval test initializer 未携带 mapping，以及四个 production
   Node oracle仍验证旧 v1/v4 generation。这些都有唯一 mechanical owner，属于 N3/N4 current-base
   debt，不改变 F391 设计。

## 3. 字段 census 与反向搜索

对 accepted `7303af9b` 执行：

```bash
git grep -o <token> 7303af9b -- \
  artifact-model artifact-identity compiler deployment runtime router scripts \
  cross-system-fixtures test-runner
git grep -l -E \
  'may_suspend|maySuspend|BoundaryCancellationContract|complete_may_effects|completeMayEffects' \
  7303af9b -- <same roots>
```

结果：

| token | occurrences | matching lines | files |
| --- | ---: | ---: | ---: |
| `may_suspend` | 580 | 534 | 117 |
| `maySuspend` | 25 | 24 | 10 |
| `BoundaryCancellationContract` | 110 | 110 | 43 |
| `complete_may_effects` | 18 | 18 | 15 |
| `completeMayEffects` | 0 | 0 | 0 |
| 五类 token union | 733 | 685 | 130 |

130 个文件按根目录分布：

| root | files |
| --- | ---: |
| `artifact-model` | 19 |
| `artifact-identity` | 13 |
| `compiler` | 46 |
| `deployment` | 7 |
| `runtime` | 38 |
| `router` | 3 |
| `scripts` | 2 |
| `cross-system-fixtures` | 1 |
| `test-runner` | 1 |

关键类型的独立反搜为：

| symbol | occurrences | files |
| --- | ---: | ---: |
| `InterfaceMethodSignature` | 83 | 20 |
| `BoundaryCallbackOperation` | 30 | 8 |
| `BoundaryOperationContract` | 131 | 42 |
| `BoundaryCancellationContract` | 110 | 43 |
| `CallbackContractOperationProjection` | 10 | 2 |
| `SourceExecutableSignature` | 43 | 9 |
| `ExecutableSignatureIr` | 31 | 16 |
| `CallableMayEffects` | 150 | 29 |
| `CallableSemanticFacts` | 68 | 24 |
| `BoundaryImplementationRequirements` | 31 | 16 |
| `PackageCallableSignature` | 141 | 38 |
| `CanonicalPublicCallableSignature` | 24 | 10 |
| `ResolvedCallTarget` | 215 | 18 |

对 `91e5475d` 重跑得到完全相同的 token数量、文件数量与文件集合；`comm -3` 为零输出。

### 3.1 A：唯一删除的 requirement / protocol summaries

| canonical owner | current producer / consumer | 终态 |
| --- | --- | --- |
| `artifact-model/src/package_unit.rs::InterfaceMethodSignature.may_suspend` | compiler core/projection硬编码或复制 `false`；legacy PackageUnit implementation-links/build与canonical PackageArtifact public type projection消费 | 删除字段；pure interface shape不拥有callee effect；strict unknown `maySuspend`拒绝 |
| `artifact-model/src/contract_types.rs::BoundaryCallbackOperation.may_suspend` | Package schema callback descriptor、source callback interface重建、runtime callback projection消费 | 删除字段；callback target在caller analysis中保守为true；旧wire拒绝 |
| `runtime/model/src/callback_projection.rs::CallbackContractOperationProjection.may_suspend` | admitted callback descriptor到runtime projection的copy/accessor | 删除copy/accessor；保留slot、method ABI、receiver ABI、params/return |
| `artifact-model/src/boundary/operation.rs::BoundaryOperationContract.may_suspend` | ServiceContract operation、compiler contract projection、deployment/runtime lane validator消费 | 删除；ServiceContract只保留code-free boundary shape |
| 同类型 `.cancellation` 与 `BoundaryCancellationContract` | `NotCancellable/Cooperative/Unsupported`由provider summary派生并驱动runtime lane | 删除字段、enum及re-export；统一runtime boundary wait/cancel/deadline owner |

直接 production owner / fixture family仍与 F395 一致，且 current 唯一新增 direct fixture 是
`deployment/src/projection/tests/operation_bindings.rs`：

- model/identity：
  `artifact-model/src/{package_unit.rs,contract_types.rs,boundary.rs,boundary/operation.rs,boundary/projection.rs,ecosystem_authoring.rs,lib.rs,package_artifact.rs,tests.rs,websocket_ingress.rs,websocket_ingress/tests.rs}`，
  `artifact-identity/src/{contract.rs,contract/normalization.rs,package/projection/implementation_links.rs,package_artifact/public_instance_tests/fixtures.rs,package_artifact/validation.rs,package_artifact/validation/public_instances.rs,package_artifact/validation/public_instances/type_normalization.rs}`；
- compiler：
  `compiler/{core/src/package_interface_methods.rs,projection/src/package_artifact/**,contract/src/**,source/src/type_resolution_model.rs,source/src/contract_dependency_test_fixture.rs,source/src/expression_type_model/contract_call_typing.rs,input/src/contract_dependencies/tests.rs,lowering/src/source_file_lowering/interface_execution_tests/contract_fixture.rs,compiled/tests/public_instance_signature_handoff.rs,driver/ecosystem_store/tests/fixtures.rs,tests/{compiler_owned_std_type_resolution.rs,file_ir_execution_type_representation.rs,service_conformance.rs,shared_fixture_lane_probes.rs,websocket_ingress.rs}}`；
- deployment：
  `deployment/src/{projection/eligibility.rs,projection/tests.rs,projection/tests/operation_bindings.rs,assembly/tests/fixtures.rs,storage/tests.rs}`；
- runtime：
  `runtime/{model/src/callback_projection.rs,eval/src/assembly_execution/**,native/src/callback_adapter.rs,boundary/src/service_value_plan_tests.rs,host/src/loader/assembly_admission/tests/**,linker/src/assembly/tests/fixtures.rs,loader/src/runtime_assembly/tests.rs,package-test/tests/support/mod.rs}`；
- tooling：
  `scripts/{check-artifact-identity-single-source.mjs,lib/runtime-execution-boundary-subjects.mjs,lib/runtime-execution-boundary-self-test.mjs}`，
  `test-runner/{src/package_schema_contract.rs,tests/package_service_contract_deployment.rs}`。

`syntax::InterfaceOperation`、`SourceInterfaceRequirementSignature`、
`InterfaceOperationIr` 当前没有 effect bit；终态保持没有，不能新增默认值或语法关键字。

### 3.2 B：同名但必须保留的 concrete facts

| fact | exact owner / copy chain | 必须保留的语义 |
| --- | --- | --- |
| `SourceExecutableSignature.may_suspend` | `compiler/source/src/contract_type_resolution.rs` 与 `contract_type_resolution/executables.rs` | body/SCC/native exact executable summary |
| `ExecutableSignatureIr.may_suspend` / `ExecutableIr.may_suspend` | `artifact-model/src/executable.rs`、`compiler/lowering/**` | FileIR concrete executable summary；FileIR v8 grammar不变 |
| `CallableMayEffects.may_suspend` | `artifact-model/src/effects.rs`、source callable fixed point | complete may-effects的一维 |
| `CallableSemanticFacts.effects` | PackageArtifact `callable_semantic_facts` | deployment effect/provenance admission |
| `BoundaryImplementationRequirements.complete_may_effects` | `artifact-model/src/boundary/projection.rs`、compiler boundary requirements | provider implementation requirement；继续与semantic facts exact比较 |
| `PackageCallableSignature.may_suspend` | PackageArtifact、projection-input、compiled handoff、canonical dependency ingest | concrete public Package callable ABI；进入canonical Local ABI/build |
| `CanonicalPublicCallableSignature.may_suspend` | `artifact-model/src/publication_abi.rs` | legacy concrete publication callable summary |
| actor/native/builtin summaries | actor declaration/identity/linked program；native/builtin registry | concrete scheduling与call analysis |
| gateway signature checks | `runtime/request/src/http_gateway_target.rs`、`runtime/eval/src/runtime_http_gateway.rs` | linked executable与Package callable exact summary equality |

public-instance validation只删除 interface-owned effect equality。下面两条必须保留：

```text
normalized interface shape == normalized concrete shape
public_signature.may_suspend == method_link.signature.may_suspend
```

concrete public summary mutation仍改变canonical Local ABI/build；ServiceContract bytes、
ServiceProtocolIdentity与ContractOperationId不能仅因provider summary变化。

### 3.3 C：current compiler target fixed point

current `ResolvedCallTarget` 仍有：

```text
ConfigIntrinsic
LocalFunction
LocalImplMethod
ActorMethod
NativeFunction
ReceiverBuiltin
DependencyPackageFunction { exact_signature: Option<PackageCallableSignature>, ... }
ContractOperation
Unknown
```

终态矩阵唯一：

| target | caller `maySuspend` | fact owner |
| --- | ---: | --- |
| local function / impl / actor | SCC exact；missing `true` | current-package fixed point |
| native / receiver builtin | registry exact；missing `true` | callable semantics registry |
| config intrinsic | `false` | intrinsic definition |
| dependency Package callable | `exact_signature.may_suspend`；missing `true` | dependency PackageArtifact |
| 新 source-internal `InterfaceMethod` | `true` | target kind |
| contract operation | `true` | target kind，不读provider contract |
| unknown/unresolved | `true` | fail closed |

current drift与唯一改法：

- `compiler/source/src/resolved_call_targets/builder.rs` 仍把已解析 interface method降成
  `Unknown(UnsupportedDynamicDispatch)`；新增携带
  `interface + method_abi_id + slot` 的 source-internal `InterfaceMethod`；
- `compiler/lowering/src/suspend_analysis.rs::call_may_suspend` 当前把dependency、contract、unknown都判
  `true`；dependency改读exact signature，interface/contract/unknown保持true；
- `compiler/source/src/callable_effects/transfer/call.rs::detached_contract_callee` 当前复制
  `contract.may_suspend`；终态直接设true，同时保留detached/return/throw shape；
- `compiler/source/src/expression_type_model.rs` 的test-effect service target当前伪造
  `PackageCallableSignature { may_suspend: contract.may_suspend }`；删除伪provider bit；
- public `CallableTargetFact`无需新增interface wire；不能表示时仍投影Unknown，而source/lowering effect
  已经保守为true。

### 3.4 D：deployment与runtime owner

`deployment/src/projection/operations.rs` 的current顺序正确，必须保留：

1. exact operation shape与callable binding；
2. exact `CallableSemanticFacts` 与 `BoundaryImplementationRequirements`；
3. boundary detached/escape/mutation/callback/stream eligibility；
4. `effects == complete_may_effects` 与provenance equality。

只删除：

- `deployment/src/projection/eligibility.rs` 的
  `effects.may_suspend != contract.may_suspend`；
- 对 `BoundaryCancellationContract::Unsupported` 的feature branch；
- 对应contract/cancellation fixture。

unknown effects、`complete_may_effects` mismatch、provenance mismatch、operation shape mismatch继续
fail closed。current新增的 `projection/tests/operation_bindings.rs` 正在锁定 complete effect mismatch，
迁移时必须保留该负例中的concrete fact。

runtime current仍在
`runtime/eval/src/assembly_execution/mod.rs` 按
`stream + cancellation + contract.may_suspend` 分叉：

- Unary终态无条件进入 `async_stream_cancel::execute_service_call`；
- ServerStream继续同一模块；
- Unsupported stream保持typed error；
- `ordinary.rs`只保留 `execute_package_direct`，删除service executor与
  `validate_ordinary_operation`；
- unary/stream/publication wait全部同时拥有ancestor/request cancellation、deadline和provider future，
  使用biased priority：cancel、expired deadline、provider；
- Ready provider允许同一poll返回，conservative caller effect不等于强制yield；
- timeout使用既有typed `DeadlineExceeded`，并cancel provider request，不能伪装成Cancelled；
- detached stream保存 `OwnedExecutionControl`，不能只保存raw token。

deadline当前只在 `runtime/request/src/execution_budget.rs` 私有字段中。N3必须给
`ExecutionControlApi` / borrowed / owned APIs增加只读
`deadline() -> Option<std::time::Instant>`，由request implementation、Host adapter与test double逐跳
转发，再用 `tokio::time::sleep_until` 唤醒pending provider。

callback/WS只删除summary equality并保留shape/target/ABI；HTTP gateway concrete checks完全不删。

## 4. F415 collection mapping reconciliation

accepted F415 exact fact流为：

```text
package.yml packages[].collection_name_mapping
  -> PackageDependency.collection_name_mapping
  -> PackageRequirement.collection_name_mapping
  -> ServiceDeployment PackageBinding.collection_name_mapping
  -> RuntimeAssembly package link collection_name_mapping
  -> linked image / linker / loader exact edge admission
  -> Host DbMetadataIr.collection_name
```

关键约束：

- `PackageRequirement` 与 `PackageBinding` 都是
  `BTreeMap<String, String>`；
- DTO是 `serde(default, skip_serializing_if = "BTreeMap::is_empty")`，所以missing/empty只有一个
  canonical wire，插入顺序不影响identity；
- compiler、generated deployment与
  `test-runner/src/package_test_assembly.rs::canonical_package_bindings` 都从上一跳 exact clone；
- requirement/binding/link drift、unknown source、partial collision、own/cross-dependency target collision及
  ambiguous active edge继续拒绝；
- mapping变化改变Package build、deployment、assembly identity value，Local ABI不变；没有schema
  generation bump。

每个 suspension 节点都要运行一个 mapping preservation negative：

```text
non-empty requirement mapping
  == deployment binding mapping
  == assembly link mapping
  == Host projection mapping
```

任一empty fallback、missing struct field、不同map或删除validator均失败。

accepted tree的 test listing 发现 F415 尚有精确 current fixture debt：

| file | missing initializers |
| --- | ---: |
| `runtime/eval/src/assembly_execution/ordinary/tests/service_error_consumer.rs` | 4 |
| `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs` | 3 |
| `runtime/eval/src/assembly_execution/ordinary/tests.rs` | 4 |
| `runtime/eval/src/assembly_execution/service_error_channel/tests.rs` | 2 |
| **合计** | **13** |

错误全部是 `PackageRequirement` / `PackageBinding` 缺
`collection_name_mapping`。N3要让同一dependency edge的requirement与binding携带相同显式值；确实没有
mapping的fixture才使用显式empty。禁止通过给model struct增加Rust default或删除F415字段来消除编译
错误。

## 5. Current generation：唯一终态

| domain | current marker / prefix / schema | terminal | 原因与 strict negative |
| --- | --- | --- | --- |
| FileIR | schema `skiff-file-ir-v8`；format v6；identity prefix `skiff-file-ir-v8:sha256` | 全部保持 | concrete executable bit保留；不能因全局 rebuild无条件换FileIR |
| Publication ABI | schema/identity v1 | 保持v1 | concrete public summary grammar不变 |
| PackageUnit | `skiff-package-unit-v1` | `skiff-package-unit-v2` | interface DTO删bit；v1 top-level严格拒绝 |
| legacy Package Local ABI | marker `skiff-package-local-abi-identity-v2`；prefix `skiff-package-local-abi-v2:sha256` | 保持v2 | preimage不含interface DTO；不能无因切代 |
| implementation links | prefix `skiff-package-implementation-links-v1:sha256` | prefix v2 | preimage interface method grammar改变；v1拒绝 |
| legacy Package build | marker `skiff-package-build-identity-v2`；prefix `skiff-package-build-v2:sha256` | marker/prefix v3 | implementation-links preimage改变；v2拒绝 |
| PackageArtifact | `skiff-package-artifact-v8` | `skiff-package-artifact-v9` | v8已被serviceCalls占用；public type DTO删bit；v8拒绝 |
| canonical Package Local ABI | marker `skiff-package-artifact-local-abi-identity-v4`；prefix `skiff-package-local-abi-v6:sha256` | marker v5；prefix v7 | public symbols grammar改变；旧marker/prefix拒绝 |
| canonical Package build | marker `skiff-package-artifact-build-identity-v7`；prefix `skiff-package-build-v9:sha256` | marker v8；prefix v10 | v7/v9已被serviceCalls占用；local ABI/links/boundary refs改变；旧值拒绝 |
| PackageSchemaType | marker `skiff-package-schema-type-identity-v1`；prefix `skiff-package-schema-type-v1:sha256` | marker/prefix v2 | callback descriptor grammar删bit；所有type records全局切代；v1拒绝 |
| PackageSchemaIndex | marker/prefix v1 | 保持v1 | index结构不变，嵌套type refs自然换值 |
| ContractOperation | marker/prefix v1 | 保持v1 | 仍只由service id + stable operation key生成 |
| ServiceContractDefinition | `skiff-service-contract-definition-v3` | v4 | operation body删provider fields；v3拒绝 |
| ServiceContract | `skiff-service-contract-v4` | v5 | operation body删provider fields；v4拒绝 |
| ServiceProtocol | marker `skiff-service-protocol-identity-v4`；prefix `skiff-service-protocol-v4:sha256` | marker/prefix v5 | protocol preimage grammar改变；v4 canonical input拒绝 |
| ServiceDeploymentInput | `skiff-service-deployment-input-v3` | 保持v3 | v3已承载exact serviceCalls binding；统一lane不加wire |
| ServiceDeployment | schema v2；marker `skiff-deployment-artifact-identity-v2`；prefix `skiff-deployment-artifact-v2:sha256` | 保持v2 | preimage结构不变；exact refs换值 |
| RuntimeAssembly | schema v2；marker `skiff-runtime-assembly-identity-v2`；prefix `skiff-runtime-assembly-v2:sha256` | 保持v2 | preimage结构不变；resolved refs换值 |
| pointer/path records | current v1/path framing | 保持 | record结构不变；嵌套identity使用新prefix |

实现必须原子切换每个提升项：没有default、dual-read、dual-write、unknown passthrough、old-hash
fallback，也不复用已占用 generation。unchanged generation的legacy rejection fixture保留原字符串，
不能被全局replace。

collection mapping不单独升generation；它必须继续进入当前canonical build/deployment/assembly
preimage，并与suspension新generation共同重算最终identity。

## 6. Router、scripts、cross-system current consumers

accepted tree的直接计数：

| root / token | occurrences | files | 终态动作 |
| --- | ---: | ---: | --- |
| `router/src` protocol v4 | 9 | 3 | canonical protocol parser/error/fixture切v5 |
| `router/tests` protocol v4 | 19 | 8 | current正例切v5；legacy rejection按语义保留 |
| `router/src` PackageArtifact v8 / build v9 | 1 / 1 | 1 / 1 | filesystem loader切v9 / v10 |
| `router/src` PackageUnit v1 | 3 | 2 | pointer parser/type切v2 |
| `scripts/lib` RuntimeAssembly v1 | 3 | 3 | current正例validator切current v2 |
| `scripts/lib` legacy build v4 | 1 | 1 | current canonical正例直接切terminal v10 |
| `scripts/lib` canonical build v9 / Local ABI v6 | 1 / 1 | 1 / 1 | terminal v10 / v7 |

exact production consumer动作：

- `router/src/router/{runtimeAssemblySnapshot.ts,runtimeAssemblyDeploymentSnapshot.ts}`：
  ServiceProtocol v4→v5；
- `router/src/protocol/runtimeProtocol.ts`：canonical runtime register/request/spawn protocol pattern、
  fixture与error text v4→v5；
- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`：
  PackageArtifact v8→v9、build v9→v10，FileIR保持v8；
- `router/src/artifacts/{pointerRecords.ts,types.ts}`：PackageUnit v1→v2；
- `scripts/check-artifact-identity-single-source.mjs`：new implementation-links/build/local/schema/protocol
  marker/prefix；
- `scripts/lib/runtime-execution-boundary-subjects.mjs`：删除
  `BoundaryCancellationContract` subject，改锁统一lane/deadline owners；
- `scripts/lib/runtime-execution-boundary-self-test.mjs`：更新synthetic positive subject，不把旧enum
  当production requirement；
- `scripts/lib/package-service-ecosystem-smoke-oracle.mjs`：
  RuntimeAssembly保持v2，build v9→v10、Local ABI v6→v7；
- `scripts/lib/package-service-authoring.mjs`：
  当前错误的positive RuntimeAssembly v1 validator→current v2；
- `scripts/lib/skiff-source-test-suite.mjs:141`：
  当前错误的positive RuntimeAssembly v1 validator→current v2；
- `scripts/lib/package-service-i02-combined-oracle.mjs`：
  positive RuntimeAssembly v1→v2、Package build v4→terminal v10；
- `cross-system-fixtures/dynamic-build-id-parity/case.json`：由terminal compiler完整再生，不做文本替换；
  concrete public `maySuspend`保留，PackageUnit/legacy build切v2/v3，canonical refs切terminal generations。

必须逐个分类，不机械替换：

- Router `filesystem-runtime-assembly-snapshot-loader` 中“rejects v1 RuntimeAssembly”是legacy负例，保留；
- Router独立legacy runtime protocol v3 rejection保持；
- scripts中服务于旧manifest/runtime domain的v1/v3/v4 rejection与error corpus保持；
- `router/src/artifacts/manifestProjection.ts` 的concrete publication
  `maySuspend`必须保留。

current Node test证据：

```text
36 tests
34 passed
2 failed
```

两个失败都属于已知N4 owner：

1. `package-service-i02-combined-oracle.mjs` 仍要求build v4，fixture已给current build v9；
2. `package-service-i02-combined.test.mjs::validI02SpawnSubmitFixtureReceipt` 按旧receipt shape写
   `serviceId`，目标对象已不存在。

这证明N4不能只改suspension字符串；必须把production positive oracles校准到current/terminal shape。

## 7. Relay G0：在 post-F415 tree 上关闭

Relay source锚点是clean Internals `960cc4bd` / `33a83817`。canonical root为：

```text
codex-relay/service/package.yml
  id: agine.ai/codex-relay
  version: 0.1.0
  packages: llm-providers, llm-api

codex-relay/service/service.yml
  id: agine.ai/codex-relay
  serviceCalls:
    - relayProxy

codex-relay/service/api.yml
  relayProxy:
    const: relay.relayProxy
    interfaces:
      - relay.CodexRelayProxyClient
```

production `serviceCall:` legacy marker为0。static receipt test在current source上：

```text
4 passed / 0 failed
node --check PASS
```

它锁定exact `serviceCalls = ["relayProxy"]`、两条service operation、30 named raw HTTP entries、
30 ingress/gateways，以及没有legacy marker。

使用accepted Skiff `7303af9b` 执行：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
node scripts/check-isolated-service-graph.mjs agine.ai/codex-relay
```

命令越过package/service/api manifest读取、dependency bootstrap、serviceCalls typed selection与contract
driver，终止于：

```text
contract validation failed:
package agine.ai/codex-relay@0.1.0 identity projection failed:
PackageArtifact is invalid:
public instance relayProxy method responsesCompleted return or suspension semantics disagree with its interface
```

因此：

```text
G0_CLOSED
```

不需要恢复旧package/API marker，不需要改Relay source、validator waiver或stable store。该失败是N0/N1
要删除interface-owned summary equality的真实正例。

## 8. 可直接派生的实现 DAG

主 Agent可直接从本result派生：

| node | 建议 task 文件 |
| --- | --- |
| N0 | `P5-F416-suspension-schema-identity-current-checkpoint.md` |
| N1 | `P5-F417-suspension-compiler-inference-projection.md` |
| N2 | `P5-F418-suspension-deployment-admission.md` |
| N3 | `P5-F419-suspension-runtime-unified-boundary.md` |
| N4 | `P5-F420-suspension-router-tooling-generation.md` |
| N5 | `P5-F421-suspension-relay-first-ecosystem-proof.md` |

这些编号在accepted tree尚未占用。每个task都必须把
`7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d` ancestry check写成启动gate。

```text
post-F415 7303af9b
  -> N0 schema/identity
  -> N1 compiler || N2 deployment || N3 runtime
  -> N4 Router/tooling/current-generation oracles
  -> N5 fresh Relay-first ecosystem proof
```

N1/N2/N3从同一个accepted N0 commit派生，production roots互不重叠；最多3个worker并行。fixture随
owner root走，不建立共享tests节点。

### 8.1 N0 — schema与identity

依赖：post-F415 gate。

exact production write roots：

```text
artifact-model/**
artifact-identity/**
```

必须修改的owner集合限于第3、5节列出的model、strict reader、normalization、identity preimage、
constants/path parser及其同目录tests/goldens。禁止修改compiler/deployment/runtime/router/scripts/
test-runner和F415 mapping语义。

内容：

- 删除interface/callback/service operation旧fields及cancellation enum；
- 保留全部concrete summaries；
- 原子切第5节所有提升generation；
- strict old-wire/prefix rejection与identity mutation矩阵；
- public concrete signature与implementation link equality继续fail closed；
- `PackageRequirement.collection_name_mapping`、`PackageBinding.collection_name_mapping`及canonical
  preimage保持。

focused tests（accepted listing数量）：

```text
cargo test --locked --manifest-path artifact-model/Cargo.toml --lib
  168 listed
cargo test --locked --manifest-path artifact-identity/Cargo.toml --lib
  120 listed
cargo test --locked --manifest-path artifact-identity/Cargo.toml --test identity_cli
  8 listed
```

负例：

- 三类legacy fields与old top-level generations拒绝；
- implementation-links v1、legacy build v2、canonical local v6、build v9、schema type v1、
  protocol v4拒绝；
- public/concrete summary mismatch仍拒绝；
- non-empty mapping mutation继续改变build，missing/empty与key insertion order仍canonical相同。

receipt：完整marker/prefix表、old-wire errors，以及provider summary mutation前后Local ABI/build/
schema type/protocol/operation/deployment/assembly identity矩阵。

### 8.2 N1 — compiler inference与projection

依赖：accepted N0。

exact production write root：

```text
compiler/**
```

仅改第3节列出的interface/callback producer、call-target builder/effect transfer、lowering、
contract/code-free projection、compiled handoff及compiler-owned fixtures。F415下列hunks必须保持：

```text
compiler/input-model/src/dependencies.rs collection_name_mapping ingest/validation
compiler/driver/generated_deployment.rs requirement mapping exact clone
compiler/driver/pipeline/mod.rs dependency -> requirement exact clone
compiler/projection-input/src/lib.rs mapping-bearing requirements
```

正例：

- 同一interface的concrete false/true都conform；
- dependency exact false/true分别传播；
- interface/service/unknown始终true；
- concrete public handoff exact；
- callback/schema/contract wire无provider bit；
- conservative true不生成synthetic runtime yield。

负例：

- missing dependency signature→true；
- public implementation mismatch拒绝；
- test-effect不能伪造contract bit；
-旧contract fixture反序列化失败；
- compiler fresh requirement的non-empty mapping不得丢失。

focused tests（accepted listing）：

```text
compiler/core package_interface                         5
compiler/source callable_effects                      83
compiler/lowering suspend                              1
compiler/projection package_artifact                  62
compiler/compiled --lib                                5
compiler/contract --lib                                6
compiler integration service_conformance              14
compiler integration file_ir_execution_type            2
```

receipt：target/effect矩阵、两种concrete PackageArtifact refs、相同ServiceContract bytes、FileIR无
interface bit/synthetic yield，以及mapping exact copy。

### 8.3 N2 — deployment admission

依赖：accepted N0；可与N1/N3并行，最终combined test读取N1 fresh artifact。

exact production write root：

```text
deployment/**
```

核心allowed files：

```text
deployment/src/projection/eligibility.rs
deployment/src/projection/operations.rs
deployment/src/projection/tests.rs
deployment/src/projection/tests/eligibility.rs
deployment/src/projection/tests/operation_bindings.rs
deployment/src/storage/tests.rs
deployment/src/assembly/tests/fixtures.rs
```

F415 `deployment/src/projection/package_closure.rs` 与
`deployment/src/assembly/resolver.rs` 的mapping equality不能删除或放宽；若fixture必须触及它们，只能
携带同一exact map。

正例：同一code-free contract可绑定provider false/true；exact build ref令deployment/assembly value
不同。

负例：unknown effects、complete effect mismatch、provenance mismatch、operation shape mismatch、
requirement/binding mapping drift继续失败。

focused tests（accepted listing）：

```text
deployment projection  19
deployment storage     13
deployment assembly    20
```

receipt：同一ServiceContractRef + 两个provider build refs的deployment/assembly diff；schema/identity
generation仍v2；mapping逐跳相同。

### 8.4 N3 — runtime统一boundary、deadline与F415 fixture适配

依赖：accepted N0；可与N1/N2并行。

exact production write root：

```text
runtime/**
```

核心allowed owners：

```text
runtime/capability-context/src/execution_control.rs
runtime/request/src/execution_budget.rs
runtime/request/src/execution_control.rs
runtime/host/src/eval_capability_adapter/execution.rs
runtime/model/src/callback_projection.rs
runtime/eval/src/assembly_execution/{mod.rs,ordinary.rs,async_stream_cancel.rs,callback_native.rs,
  websocket_contract_plan.rs,projection.rs,boundary_materialization/tests.rs,ordinary/tests.rs,
  ordinary/tests/service_error_consumer.rs,ordinary/tests/source_inline_effect_e2e.rs,
  service_error_channel/tests.rs}
runtime/native/src/callback_adapter.rs
runtime/{boundary,linker,loader,host,package-test}/**/*fixture*
```

F415 production mapping owners
`runtime/linked-program/src/shared_image.rs`、
`runtime/linker/src/assembly.rs`、
`runtime/loader/src/runtime_assembly/graph_validation.rs` 与
`runtime/host/src/loader/active_assembly_context.rs` 的exact mapping validation/projection禁止删除。

正例：

- Ready unary同poll返回；
- Pending provider被provider/cancel/deadline唤醒；
- cancel/deadline同时ready时cancel优先；
- deadline得到typed DeadlineExceeded并cancel provider；
- stream item、terminal与publication wait都覆盖cancel/deadline；
- callback/WS summary不同但shape一致时接受；
- 13个F415 fixture initializer携带exact mapping并编译。

负例：

- shape/target/ABI mismatch仍拒绝；
- deadline不能降成Cancelled；
- timeout后provider cancel signal必须出现；
- stream task/lease归零；
- Unsupported stream/callback保持typed error；
- mapping drift/collision validators继续fail closed。

focused listing / current结果：

```text
capability execution_control       1 listed
request execution_budget           5 listed
model callback_projection          3 listed
eval assembly_execution            current compile FAIL: exact 13 mapping initializers
native callback_adapter            7 listed
linker assembly                    30 listed
loader runtime_assembly            17 listed
host assembly_admission            30 listed
```

N3验收时 `eval assembly_execution` 必须从compile failure变成完整listed+pass，并运行其余selectors。

### 8.5 N4 — Router、tooling、goldens与current positive oracles

依赖：N1/N2/N3 combined integration。

exact tracked write set：

```text
router/**
scripts/check-artifact-identity-single-source.mjs
scripts/lib/runtime-execution-boundary-subjects.mjs
scripts/lib/runtime-execution-boundary-self-test.mjs
scripts/lib/package-service-ecosystem-smoke-oracle.mjs
scripts/lib/package-service-authoring.mjs
scripts/lib/skiff-source-test-suite.mjs
scripts/lib/package-service-i02-combined-oracle.mjs
scripts/tests/artifact-identity-validation.test.mjs
scripts/tests/runtime-execution-boundary-checker.test.mjs
scripts/tests/package-service-authoring.test.mjs
scripts/tests/skiff-source-test-suite.test.mjs
scripts/tests/package-service-i02-combined.test.mjs
scripts/tests/package-service-bootstrap-oracle-handoff.mjs
scripts/tests/package-service-ecosystem-http-fixture.test.mjs
scripts/tests/package-service-ecosystem-smoke-real.test.mjs
scripts/tests/helpers/package-service-ecosystem-smoke-fixtures.mjs
scripts/tests/platform-source-transport-combined.test.mjs
scripts/tests/run-skiff-tests-error-evidence.test.mjs
scripts/tests/check-artifact-identity-single-source.test.mjs
scripts/tests/verify.test.mjs
cross-system-fixtures/dynamic-build-id-parity/case.json
test-runner/tests/package_service_contract_deployment.rs
```

`test-runner/src/package_test_assembly.rs::canonical_package_bindings` 不在write set；它的mapping exact
clone是F415 gate。test-runner test fixture可删除旧contract/interface fields，但concrete
`may_suspend`与mapping断言保留。

正例：

- Router direct join/filesystem loader读取terminal fresh records；
- identity single-source checker通过；
- PackageUnit v2 pointer正例；
- source-suite与authoring production validators接受RuntimeAssembly v2；
- I02使用RuntimeAssembly v2与build v10，并适配current receipt shape；
- dynamic fixture由terminal compiler再生。

负例：

- protocol v4、PackageArtifact v8、canonical build v9、PackageUnit v1、legacy build v2及old fields拒绝；
- current legacy rejection字符串保留；
- path escape、duplicate key、mapping drift负例保留。

focused evidence：

```text
Router five-file vitest list: 164 tests
current Node five-file group: 36 total / 34 pass / 2 known N4 failures
test-runner package_service_contract_deployment: 24 listed
```

终态命令至少包括：

```bash
node --test \
  scripts/tests/artifact-identity-validation.test.mjs \
  scripts/tests/package-service-authoring.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/tests/runtime-execution-boundary-checker.test.mjs \
  scripts/tests/skiff-source-test-suite.test.mjs
node scripts/check-artifact-identity-single-source.mjs
pnpm --filter @skiff/router exec vitest run \
  tests/compilerGeneratedManifestCompatibility.test.ts \
  tests/dynamic-build-id-parity.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts \
  tests/protocol.test.ts \
  tests/artifacts.test.ts
pnpm --filter @skiff/router exec tsc --noEmit --pretty false
```

receipt：fresh fixture producer commit/tree、record paths、schema/prefix集合、Router/Node exact count、old
canonical generation rejection与legacy rejection inventory。

### 8.6 N5 — fresh Relay-first ecosystem proof

依赖：N4。

Skiff production write set：

```text
∅
```

Internals production write set：

```text
∅
```

fresh artifacts、source mirror与receipt只写task-owned temporary root。current Internals的
`codex-relay/service/service-api-receipt.test.mjs` 有两个positive protocol v4 validators；terminal
proof需要v5。若N5允许同步checked-in evidence oracle，唯一non-production tracked write set是：

```text
codex-relay/service/service-api-receipt.test.mjs
```

并须在Internals独立commit。若主 Agent要求N5严格tracked-write `∅`，直接另派生
`P5-F421A-relay-suspension-receipt-oracle.md`，只授权上述一个test文件；这不是设计决策或production
migration。

fresh rebuild顺序：

```text
std
  -> llm-api
  -> llm-providers
  -> Relay
  -> {Agent, http-session, track, Aihub, Account, Registry}
  -> Agine
```

Relay必须先满足：

- operation set精确为
  `relayProxy.responsesCompleted`、
  `relayProxy.responsesCompletedResult`；
- 两个concrete `PackageCallableSignature.may_suspend == true`；
- interface method wire无`maySuspend`；
- ServiceContract operation无`maySuspend`/`cancellation`；
- ContractOperationId仍精确为：
  - `skiff-contract-operation-v1:sha256:b62d89d553cc0607b2627b047d2a5ab4665c70f05f900babbce249def47099ef`
  - `skiff-contract-operation-v1:sha256:51fa082dd0d33b09f45e4900805c28801cb3108b4eac813697e66e5f8a6b007d`
- generations：
  PackageArtifact v9、Local ABI v7、build v10、ServiceContract v5、protocol v5、
  ServiceDeployment/RuntimeAssembly v2；
- Relay package/dependency mapping fact与fresh assembly link exact一致。

之后重算所有current interface/concrete pairs与callback records；不能复用F395的99 pair分类作为新
golden。每个callback-interface record必须无provider bit，implementor summary mutation不改变type ID。

负例：任一old field/prefix、Relay operation count非2、concrete summary非true、operation ID变化、
consumer旧protocol、mapping drift、old lock/store或validator waiver都使proof失败。

receipt必须记录每步input commit/tree、命令、stdout JSON、artifact paths与SHA-256、schema/prefix、
operation/pair/callback列表、mapping与consumer exact refs。不能使用stable/live输出替代。

## 9. 跨节点负矩阵

| invariant | N0 | N1 | N2 | N3 | N4 | N5 |
| --- | --- | --- | --- | --- | --- | --- |
| old interface/callback/contract fields拒绝 | owner | compiler fixture | deployment fixture | runtime fixture | parser/golden | fresh records |
| concrete summary保留且exact | identity | inference/handoff | admission | execution/gateway | fixture | Relay true |
| old generations/prefixes拒绝 | owner | fresh output | exact refs | loader refs | parser | ecosystem scan |
| interface false/true均conform | identity | owner | bind | shape-only runtime | golden | real pairs |
| dependency missing summary→true | — | owner | — | — | — | fresh negative |
| complete effects/provenance mismatch拒绝 | model | producer | owner | exact runtime artifact | checker | tamper |
| code-free contract对provider mutation稳定 | identity | projection | bind | same lane | fixture | Relay mutation |
| cancel/deadline/provider priority | — | no synthetic yield | — | owner | boundary checker | real trace |
| mapping exact逐跳且collision拒绝 | preimage | requirement | binding | link/Host | fixture | ecosystem |
| legacy rejection不被机械替换 | golden | fixture | fixture | fixture | owner | final scan |

## 10. 合流顺序与证据失效

合流顺序唯一：

1. integration HEAD必须是`7303af9b` descendant；
2. N0原子合流；
3. N1/N2/N3分别从同一N0 base派生；建议按N1→N2→N3合流并在combined tree重跑三者tests；
4. N4从combined tree派生并再生fresh fixtures；
5. N5从accepted N4 tree执行Relay-first proof；
6. 跨Skiff/Internals的task分别commit；push仍需用户明确授权。

以下任一情况使相应证据失效：

- implementation HEAD不再包含exact `7303af9b`，或F415 mapping chain/hunks变化：全部节点失效；
- N0 marker/prefix/schema/preimage或strict reader有任何amend：N1–N5全部重跑；
- N1 call-target/effect/projection变化：N1、combined N2 proof、N4、N5重跑；
- N2 eligibility/binding变化：N2、N4、N5重跑；
- N3 wait/cancel/deadline或mapping fixture变化：N3、N4 boundary checker、N5重跑；
- N4 parser/oracle/fixture变化：N4、N5重跑；
- Internals不再包含`960cc4bd`，或Relay四个owner文件变化：G0、static receipt、N5重跑；
- Cargo.lock、pnpm lock、Rust/Node toolchain或shared target provenance变化：受影响listing/pass count不再是
  acceptance evidence，须在final clean tree与owned/isolated target重跑；
- 使用stable/live、old artifact store、old lock、手工patched Relay mirror或validator waiver生成的
  输出无效。

当前 shared target上的`runtime/eval` compile failure是accepted source的13个明确fixture omissions；
修复前不能把其它runtime selector的listed count冒充N3通过。

## 11. 最终判定

current code没有暴露会改变F391语义的冲突。G0已在post-F415 accepted tree上关闭，generation、owner、
mapping preservation、tooling drift、Relay proof与合流顺序均有唯一任务合同。

```text
TASK_EXECUTABLE
```
