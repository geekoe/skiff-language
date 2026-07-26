# P5-F395 Inferred suspension implementation audit result

状态：Complete；`TASK_EXECUTABLE`。F391 A–E 足够形成唯一、无 production write-set
重叠的实现 DAG；没有新的语言语义决策题。

## 1. 结论与唯一实现选择

本审计冻结以下实现，不再把它们留给后继自行选择：

1. interface requirement、callback-interface schema、ServiceContract operation 都不再携带
   callee `maySuspend`；ServiceContract 同时删除由该位派生的 `cancellation`。
2. concrete executable、concrete Package callable、callable semantic facts 和
   `BoundaryImplementationRequirements.completeMayEffects` 继续携带精确推断值。
3. dependency Package call 读取 exact published `PackageCallableSignature.maySuspend`；
   interface/`any I`/未知动态调用保守为 `true`；service target 种类本身令 caller
   `maySuspend=true`，不读取 provider contract。
4. provider summary 已经存在于 PackageArtifact 的 concrete callable/semantic facts，并由
   ServiceDeployment 的 exact PackageArtifactRef 绑定；本次不新增一份 deployment wire copy。
   runtime service boundary 选择统一 lane，因此也不读取该 fact 做 ordinary/async 分叉。
5. 所有 unary service call 走现有 async boundary materialization lane；Ready future 可以在首次 poll
   同步完成，只有真实 Pending 才让出。pending unary/stream 一律同时观察 ancestor/request
   cancellation 与 request deadline。
6. 采用上述统一 lane 后，ServiceDeploymentInput、ServiceDeployment、RuntimeAssembly 的结构与 identity
   framing 保持 v2；它们会因 exact PackageArtifactRef/ServiceContractRef 改变而产生新 identity value，
   但不提升 generation。

这同时回答了任务中“从 ServiceContract 迁到 implementation/deployment metadata 的 runtime fact”：
不是新增第三份 summary，而是保留既有 PackageArtifact canonical fact 和 deployment exact reference。
若后续性能优化确实要在 assembly 中内联它，必须另开 schema/identity 设计，不能在本 DAG 偷加字段。

## 2. 审计锚点、边界与基线偏差

| 树 | commit | tree | 用途 |
| --- | --- | --- | --- |
| 本审计 worktree | `caa2f759eaecd21c537a58eac7ff78dda06013af` | `a3bd663c7b79abba47673fbf0ce625c715a56322` | F391 A–E 字段、schema、compiler、deployment、runtime、Router 的 canonical source baseline |
| `/Users/geek/workspace/skiff` main | `305882351b1e3ea644f1aef3bbc5a1477ab15858` | `7122a1c89a21de83e3b861a3148fcee6ff8317bc` | current service-only authoring 参考；不含 phase-05 canonical schema/runtime 链，不能替代上一行 |
| Internals | `5861c13f3a92b7fb56a5cfa689e46f5d0462a02d` | `867c99c155386299e7dbb8b4fed95cee2427ba84` | current production ecosystem source |
| skiff-packages | `5defc94161cee14def1a6bbb340308004e65b741` | `d8763acf82e0320135704297f2419bf5cd3558e5` | current official package source |

审计开始时四棵 production tree 均 clean。本任务没有修改 Skiff、Internals 或 skiff-packages，
没有访问 stable instance、live store 或外部服务。所有探针只写 `/tmp`。

这里存在一个必须前置处理、但不改变 F391 语义的 integration gate：

- phase-05 baseline 的 canonical compiler 仍要求 service root 有 `package.yml`，并从旧 authoring graph
  识别 service operation；
- current Internals 已把 service id/version/package dependencies 收进 `service.yml`，删除五个 service
  root 的 `package.yml`，Relay `api.yml` 也不再写旧 `serviceCall` 标记；
- current Skiff main 能识别 service-only authoring，却只有较早的 PackageArtifact/ServiceContract
  generation，不能用于验证本任务的 phase-05 field graph。

因此后继实现必须基于一个同时包含 phase-05 canonical source 与 current service-only authoring 的
集成提交。当前任一单独 HEAD 都不能产出诚实的“current Relay + F391”receipt。这个事实不是
`TASK_NOT_EXECUTABLE`：字段语义和代码改法均唯一；它是下文 `G0` 的 source-base gate。

### 2.1 只读枚举与诊断探针

- 对 `artifact-model`、`artifact-identity`、`compiler`、`deployment`、`runtime`、`router`、
  `scripts`、`cross-system-fixtures` 的 `may_suspend` / `maySuspend` /
  `BoundaryCancellationContract` / `complete_may_effects` 搜索共得到 677 个命中；下文把它们按
  A–E owner 和 fixture family 全部分域。不能用全局删除替代这些分类。
- 用 phase-05 compiler、fresh `/tmp` artifact store 和 current Internals source 做了诊断编译。
  为跨 source/compiler skew，只在临时副本中做 effect-neutral 适配：补显式 object target、
  把 current dependency `.` address 改回等价 `/` address、恢复旧 manifest state/API closure，
  并移除无 method 的新 `ErrorPayload` conformance。未改任何 interface/impl method body或等待点。
- 该探针 current Agent 精确发出 73 个 pair：39 个 `maySuspend=false`、34 个 `true`，与 F382
  相同。诊断 PackageArtifact 为 build
  `skiff-package-build-v8:sha256:f6bd2944b1d718623642a2325dbaa5d908399d031536c837adaed20fb1722a3a`。
- 同一隔离链的 std、http-session、llm-api、llm-providers、Agent 共 156 个
  PackageSchemaType record，`callbackInterface` record 为 0；F382 的真实 Relay 也为 0。
  这些是审计证据，不是 F391 新 generation receipt。
- current Relay 对 phase-05 CLI 会在读取 source 前因缺 `package.yml` 失败。没有跳过 public-instance
  validator，也没有把 F382 waiver probe 当作验收。

## 3. F391 A：删除 interface requirement 与 callback summary

### 3.1 Canonical model owner

| owner | 当前字段/用途 | 唯一改法 | identity / strict owner |
| --- | --- | --- | --- |
| `artifact-model/src/package_unit.rs::InterfaceMethodSignature.may_suspend` | strict camelCase wire `maySuspend`；同一 DTO 同时进入 legacy `PackageUnit::TypeExport.interface_methods` 与 canonical `PackageArtifact::PackageLocalAbiSymbol::Type.interface_methods` | 删除字段；不加 default、不 dual-read | PackageUnit v2；legacy implementation-links/build；PackageArtifact local ABI/build |
| `artifact-model/src/contract_types.rs::BoundaryCallbackOperation.may_suspend` | `ContractTypeDescriptor::CallbackInterface.operations[*]` 的 callee summary | 删除字段 | PackageSchemaType marker/prefix v2；所有旧 callback descriptor 的该 unknown field fail closed |
| `runtime/model/src/callback_projection.rs::CallbackContractOperationProjection.may_suspend` 与 accessor | 把 callback schema summary复制进 admitted local method projection | 删除字段、constructor copy 和 accessor | runtime-only projection，不另升 public schema |

`syntax::InterfaceOperation`、`SourceInterfaceRequirementSignature`、`InterfaceOperationIr` 当前本来就没有
该字段，保持没有；不得加语言关键字或中间层默认值。

### 3.2 所有 producer、copy、normalize 与 validator

| 文件 / 函数 | 当前行为 | 实现后 |
| --- | --- | --- |
| `compiler/core/src/package_interface_methods.rs::package_interface_method_signature` | 从 `InterfaceOperationIr` 构造 DTO 时硬编码 `false` | 构造纯 receiver/params/return/flags shape |
| 同文件 `instantiate_interface_method_signature` | destructure/copy `may_suspend` | 删除 destructure/copy；type-param substitution只处理 shape |
| 同文件 `normalize_package_interface_method_signature` | normalize 时复制 bit | 删除 bit copy |
| `compiler/projection/src/package_artifact/callables/mod.rs::project_implementation_types` | 第二条 projection path 对每个 FileIR interface operation硬编码 `false` | 只投影 method shape |
| `compiler/projection/src/package_artifact/visible_types.rs::projection_visible_interface_method_signature` | visible type normalize 时复制 bit | 删除 copy；相邻 executable normalize 的 bit 必须保留 |
| `compiler/source/src/type_resolution_model.rs::service_api_interface` | 从 callback schema operation 重建 interface method并复制 bit | 重建纯 shape |
| 同文件其它 Package interface reconstruction/normalization | 间接携带 DTO bit | 全部随 DTO 删除；不能从 concrete executable 回填 requirement |
| `artifact-identity/src/package/projection/implementation_links.rs::PackageImplementationLinksIdentityProjection` 与 `package.rs::package_implementation_links_identity` | 整体序列化包含 interface method DTO 的 implementation links | DTO grammar 随字段删除并切 implementation-links v2；不得复用 v1 hash framing |
| `artifact-identity/src/package_artifact/validation/public_instances/type_normalization.rs` | normalize interface DTO时复制 bit | 删除 copy |
| `artifact-identity/src/package_artifact/validation/public_instances.rs::validate_public_instance_method_signature` | interface、method link、public signature 三方比较 bit | interface只参与 receiver/params/return；仍直接比较 `public_signature.may_suspend == method_link.signature.may_suspend` |
| `compiler/source/.../interfaces/conformance.rs` | 已经只比较 requirement-owned shape | 保持；新增 true/false concrete implementations 都 conform 的锁定测试 |

public-instance validator 不能简单删除全部 suspension equality。应把当前 return 条件改成：

```text
normalized(interface return) == normalized(concrete return)
package(public return) == interface return
public_signature.may_suspend == method_link.signature.may_suspend
```

也就是说 interface 不拥有 effect，但 public concrete signature 仍必须与其 exact implementation link
fail closed。

### 3.3 Callback schema 到 runtime 的完整链

| 文件 / 函数 | 当前 consumer | 迁移 |
| --- | --- | --- |
| `compiler/projection/src/package_artifact/schema.rs::SchemaBuilder::project_descriptor` | interface method bit -> `BoundaryCallbackOperation` | 只投影 ordered operation names、params、return |
| `artifact-identity/src/contract.rs::{normalize_schema_descriptor, package_schema_type_id}` | canonical descriptor进入 PackageSchemaType preimage | descriptor grammar删除 bit并提升 marker/prefix；shape排序保持 |
| `compiler/source/src/type_resolution_model.rs::service_api_interface` | callback schema -> source interface method | 不再制造 effect |
| `runtime/model/src/callback_projection.rs::CallbackContractProjection::new` | contract operation -> runtime operation projection | 不再复制 bit |
| `runtime/eval/src/assembly_execution/callback_native.rs::validate_adapter_preimage` | local executable与 callback operation summary equality | 删除 summary equality；保留 slot、method ABI、receiver ABI、parameter/return type与 exact executable target |
| `runtime/native/src/callback_adapter.rs` | 构造/测试 adapter projection | fixture删除 contract bit；实际 concrete target调用不变 |

callback-interface 调用的 caller analysis 由 C 类显式 interface/unknown target 保守为 `true`；runtime
method table 仍调用 exact executable，但不把该 executable summary提升成 callback protocol保证。

### 3.4 A 类 direct fixtures

下列 direct field-carrying fixture/golden 必须随所属 production owner改，不能遗落：

- model/identity：
  `artifact-model/src/{tests.rs,package_artifact.rs}`，
  `artifact-identity/src/package/projection/implementation_links.rs`，
  `artifact-identity/src/package_artifact/{public_instance_tests.rs,public_instance_tests/fixtures.rs}`，
  `artifact-identity/src/package_artifact/validation/public_instances/type_normalization.rs`；
- compiler：
  `compiler/core/src/package_interface_methods.rs` tests，
  `compiler/projection/src/package_artifact/{callables/normalization.rs,callables/surface.rs,tests/fixtures.rs,export_links/tests/fixtures.rs,schema.rs}`，
  `compiler/compiled/tests/public_instance_signature_handoff.rs`；
- callback runtime：
  `runtime/model/src/callback_projection.rs` tests，
  `runtime/eval/src/assembly_execution/{callback_native.rs,boundary_materialization/tests.rs}`，
  `runtime/native/src/callback_adapter.rs`，
  `runtime/host/src/loader/assembly_admission/tests/{full_chain.rs,execution/artifacts.rs}`。

## 4. F391 B：必须保留的 concrete summary

以下字段即使也叫 `maySuspend`，都不是 A/D 类旧 protocol field：

| canonical fact | owner / copy chain | 当前用途；实现要求 |
| --- | --- | --- |
| `SourceExecutableSignature.may_suspend` | `compiler/source/src/contract_type_resolution.rs`；`contract_type_resolution/executables.rs::{build_executable_signature_facts,build_executable_signature_facts_from_may_suspend}` | body/SCC/native facts的 exact source executable summary；保留 |
| `ExecutableSignatureIr.may_suspend`、`ExecutableIr.may_suspend` | `artifact-model/src/executable.rs`；`compiler/lowering/src/{suspend_analysis.rs,executable_declaration_lowering.rs,function_lowering.rs,lowered.rs,source_file_lowering.rs}` | FileIR executable和linked code summary；FileIR v8 grammar不变 |
| `CallableMayEffects.may_suspend` | `artifact-model/src/effects.rs`；source callable effect fixed point与provenance | complete may-effects的一维；保留 |
| `CallableSemanticFacts.effects` | PackageArtifact `callable_semantic_facts` | deployment eligibility、effect/provenance校验；保留 |
| `BoundaryImplementationRequirements.complete_may_effects` | `artifact-model/src/boundary/projection.rs`；`compiler/projection/.../boundary/requirements.rs` | provider implementation requirement，不是ServiceContract；保留且继续与semantic facts exact比较 |
| `PackageCallableSignature.may_suspend` | `artifact-model/src/package_artifact.rs`；`compiler/projection-input/src/package_callable_signatures.rs`；`compiler/driver/source_compile/canonical_dependencies.rs`；compiled handoff；projection `attach_canonical_signatures` | concrete public Package callable ABI与 dependency exact summary；进入 canonical Local ABI/build |
| `CanonicalPublicCallableSignature.may_suspend` | `artifact-model/src/publication_abi.rs` | legacy publication仅表示 concrete callable期间保留；Publication ABI v1不变 |
| actor public/executable summaries | `artifact-model/src/actor_declaration.rs::ActorPublicMethodIr`；`artifact-identity/src/actor.rs`；`runtime/linked-program/src/linked.rs::{LinkedActorPublicMethod,LinkedExecutable}` | actor concrete scheduling/ABI与implementation identity；保留 |
| native/builtin summaries | `artifact-model/src/{native_signature.rs,builtin_receiver_ops.rs}`；runtime native registry/eval context | fixed native/builtin wait fact；保留 |
| gateway concrete signature checks | `runtime/request/src/http_gateway_target.rs`；`runtime/eval/src/runtime_http_gateway.rs` | linked concrete executable必须与 Package callable signature一致；保留 |

`compiler/compiled/src/projection_input.rs`、source
`SourceExecutableSignature::package_callable_signature`、compiled
`package_callable_signatures` 与 projection callable attachment 构成唯一 public summary source。
public-instance summary不得从 interface、FileIR requirement或 service descriptor 补值。

Package callable summary mutation仍有以下 identity语义：

- `PackageCallableId` 只按 stable public path生成，保持；
- canonical Package Local ABI/build改变；
- private concrete executable只有 build改变；
- ServiceContract bytes、ServiceProtocolIdentity、ContractOperationId均不因 provider summary单独改变；
- deployment/assembly因 exact provider PackageArtifactRef改变而重算。

B 类保留 fixture 包括：

- `artifact-model/src/{effects.rs,executable.rs,native_signature.rs,builtin_receiver_ops.rs,publication_abi.rs,actor_declaration.rs}`；
- `artifact-identity/src/{actor.rs,package_artifact.rs,tests/publication_validation.rs,tests/operation.rs}`；
- `compiler/lowering` suspension tests、`compiler/source/src/contract_type_resolution/tests*`、
  `compiler/projection-input/src/package_callable_signatures.rs`、
  `compiler/tests/file_ir_execution_type_representation.rs`；
- `runtime/eval/src/{actor_executor.rs,eval_context.rs,program_execution.rs,program_stream.rs}`、
  `runtime/native/src/registry/tests.rs`、`runtime/linker/src/linker/{file_conversion.rs,link_diagnostics.rs}`、
  `runtime/linked-program/src/{linked.rs,resolver.rs}`。

## 5. F391 C：按 call target 种类传播

### 5.1 唯一 target 矩阵

| `ResolvedCallTarget` | caller `maySuspend` | fact source |
| --- | ---: | --- |
| `LocalFunction` / `LocalImplMethod` / `ActorMethod` | fixed point exact；缺值 `true` | current-package SCC |
| `NativeFunction` | exact；缺 registry fact `true` | `native_callable_semantics` |
| `ReceiverBuiltin` | exact；缺 registry fact `true` | `builtin_receiver_callable_semantics` |
| `ConfigIntrinsic` | `false` | intrinsic definition |
| `DependencyPackageFunction` | `exact_signature.may_suspend`；missing summary `true` | dependency PackageArtifact public callable |
| 新增 `InterfaceMethod` | `true` | target种类本身 |
| `ContractOperation` | `true` | target种类本身，绝不读provider contract |
| `Unknown` / 无法解析 | `true` | fail closed |

新增 source-internal variant应携带足够的诊断identity：

```text
ResolvedCallTarget::InterfaceMethod {
  interface: InterfaceInstantiationRef,
  method_abi_id: String,
  slot: u32,
}
```

`TypeResolutionModel::any_interface_method_signature` 已返回
`AnyInterfaceMethodResolution { interface, slot, method_abi_id, params, return }`；
`compiler/source/src/resolved_call_targets/builder.rs` 当前却把它降成
`Unknown(UnsupportedDynamicDispatch)`。后继应在这里显式构造新 variant。
compiled target/provenance artifact无需新增公开 enum：若现有 `CallableTargetFact` 没有 interface 分支，
把它投影为 `CallableTargetFact::Unknown`，同时 source/lowering effect已保守为 true。

### 5.2 所有传播 consumer

| 文件 / 函数 | 当前问题 | 唯一修改 |
| --- | --- | --- |
| `compiler/lowering/src/suspend_analysis.rs::call_may_suspend` | dependency、contract、unknown统一 `true` | dependency读 exact signature；contract/interface/unknown `true`；local/native/builtin不变 |
| 同文件 legacy `package_or_service_call_may_suspend` | facts缺失时按root保守 | 只作为缺 typed fact 的 fail-closed fallback，不覆盖 exact dependency result |
| `compiler/source/src/callable_effects/transfer/call.rs` package path | 已消费 dependency semantic facts | 保持 exact；缺/unknown仍true |
| 同文件 `detached_contract_callee` | `state.effects.may_suspend = contract.may_suspend` | 因 target 为 service call直接设 `true`；detached provenance/return/throw shape继续来自code-free contract |
| `compiler/source/src/expression_type_model.rs` compiler test-effect service target | 伪造 `PackageCallableSignature { may_suspend: contract.may_suspend }` | service target view不承载provider summary；caller effect独立按 target=true |
| source/lowering compiled handoff | interface当前只剩 Unknown provenance | 接受显式 source target，并在公开 artifact projection保守归类 |

### 5.3 C 类正负测试

必须同时锁定：

- dependency Package callable `false -> false`、`true -> true`；
- missing dependency exact signature -> `true`；
- 同一个 interface requirement 分别绑定 concrete `false`/`true`，conformance都通过；
- 静态 concrete/public-instance direct target分别传播其 exact值；
- `any I` 与已知 interface method target始终 `true`；
- service target面对 provider `false`/`true` 都是 caller `true`；
- 修改/删除旧 contract bit 不再影响 source effect；
- conservative `true` 不生成新 IR wait/yield opcode，Ready concrete执行不产生额外交错。

主要 direct fixture owner：

- `compiler/source/src/{callable_effects/tests.rs,resolved_call_targets*,expression_type_model.rs,contract_dependency_test_fixture.rs}`；
- `compiler/lowering/src/{suspend_analysis.rs,source_file_lowering.rs,source_file_lowering/interface_execution_tests/contract_fixture.rs}`；
- `compiler/input/src/contract_dependencies/tests.rs`；
- `compiler/tests/{service_conformance.rs,std_package_imports.rs,shared_fixture_lane_probes.rs}`。

## 6. F391 D：ServiceContract、deployment 与 runtime

### 6.1 删除的 protocol fields

| owner | 当前字段/consumer | 实现后 |
| --- | --- | --- |
| `artifact-model/src/boundary/operation.rs::BoundaryCancellationContract`、`artifact-model/src/lib.rs` re-export | `NotCancellable` / `Cooperative` / `Unsupported`，由 callee summary派生 | 删除整个 enum及 re-export |
| 同文件 `BoundaryOperationContract.cancellation` | ServiceContract operation wire、deployment/runtime lane选择 | 删除 |
| 同文件 `BoundaryOperationContract.may_suspend` | provider summary protocol copy | 删除 |
| `compiler/projection/src/package_artifact/boundary/types.rs::project_operation_contract` | `signature.may_suspend -> cancellation` 并复制 summary | 只投影 parameters/return/stream/callback/effect guarantee |
| `compiler/contract/src/definition.rs::ServiceContractDefinition.operations` 与 `{projection.rs,tests.rs}` | strict definition直接嵌入 operation DTO；fixture带上述字段 | definition v4仍嵌入 strict code-free operation shape；projection不再补 effect |
| `compiler/input` 与 source contract dependency fixtures | ingest old operation fields | 新 strict shape |
| `artifact-identity/src/contract/normalization.rs::normalize_contract_operation_contract` 与 `contract.rs::ServiceProtocolIdentityProjection` | normalize完整 operation DTO并进入protocol preimage | 只 normalize code-free shape；descriptor grammar无旧字段，generation v5 |

`BoundaryEffectGuarantee` 保留；它承诺跨 boundary 的 detached/alias shape，不是 provider
`maySuspend`。`BoundaryStreamContract`、`BoundaryCallbackContract` 也保留。

### 6.2 Deployment 不丢 provider exact fact

`deployment/src/projection/operations.rs` 的顺序应继续是：

1. exact operation shape 与 callable binding匹配；
2. 从绑定 PackageArtifact读取 `CallableSemanticFacts` 和
   `BoundaryImplementationRequirements`；
3. `validate_boundary_eligibility` 检查 detached、escape、mutation、callback/stream shape；
4. `validate_callable_facts` exact比较
   `effects == requirements.complete_may_effects` 与 provenance。

只删除：

- `deployment/src/projection/eligibility.rs::validate_effects` 中
  `effects.may_suspend != contract.may_suspend`；
- `validate_contract_features` 对
  `BoundaryCancellationContract::Unsupported` 的分支；
- 相应 cancellation fixture。

unknown effect、complete may-effects mismatch、provenance mismatch仍必须失败。这样 provider exact summary
仍被 deployment admission验证并通过 exact PackageArtifactRef绑定，只是不再与协议位比较。

### 6.3 Runtime 统一 boundary lane

`runtime/eval/src/assembly_execution/mod.rs` 当前按
`stream + cancellation + contract.may_suspend` 在 `ordinary` 与 `async_stream_cancel` 间分叉。终态：

- `BoundaryStreamContract::Unary` 无条件调用
  `async_stream_cancel::execute_service_call`；
- `ServerStream` 继续调用同一模块；
- `Unsupported stream` 继续 typed unsupported error；
- `ordinary.rs` 只保留 `execute_package_direct`；删除 service boundary executor与
  `validate_ordinary_operation`；
- `async_stream_cancel.rs::execute_provider_unary` 成为 unary 唯一 materialization、ActivationContext、
  provider error export/import 和 result materialization路径。

`async_stream_cancel.rs` 内所有旧 policy consumer 必须一起移除，而不是只改入口 match：

- `await_provider_unary` 删除 `cancellation_contract` 参数，始终选择 request cancellation /
  deadline / provider future；
- `start_provider_stream` 删除 `Unsupported` 预检，`ProviderStreamTask` 删除
  `cancellation_contract` 字段并保存 owned execution control；
- `await_provider_stream_terminal` 始终选择 consumer cancellation、request cancellation、deadline与
  provider terminal；
- `publish_provider_terminal` 始终在 publication wait 中选择 request cancellation与deadline。

统一 lane 不等于强制 scheduler yield：

```text
caller materialize
  -> poll cancellation
  -> poll expired deadline
  -> poll provider future
       Ready: 同一 poll 返回，不让出
       Pending: runtime自然挂起；由provider/cancel/deadline唤醒
```

`tokio::select!` 使用 `biased`，优先级固定为 ancestor/request cancellation、已到 deadline、provider
Ready。这样 cancellation 与 deadline 同时 ready 时保持既有 cancellation优先；deadline不是伪装成
cancel。

### 6.4 Deadline 与 detached stream 的现有缺口

当前 `runtime/request/src/execution_budget.rs::ExecutionBudget.deadline` 是 private，只有执行指令时的
`poll_execution_budget()` 能发现超时。pending provider future 若不自行 wake，deadline到达后不会被
poll，这是删除 contract lane 分叉时必须同时修复的真实 correctness gap。

唯一改法：

- 给 `runtime/capability-context/src/execution_control.rs` 的
  `ExecutionControlApi`、`ExecutionControl`、`OwnedExecutionControlApi`、`OwnedExecutionControl`
  增加 `deadline() -> Option<std::time::Instant>`；
- `runtime/request/src/execution_budget.rs` 提供只读 getter，
  `runtime/request/src/execution_control.rs` 的 borrowed/owned implementation转发；
- `runtime/host/src/eval_capability_adapter/execution.rs` 与
  `runtime/eval/src/assembly_execution/ordinary/test_runtime.rs` 的 adapter/test double同步实现；
- unary wait 对 deadline建 `tokio::time::sleep_until`；wake 后调用现有
  `poll_execution_budget()`，从而产生既有 typed `DeadlineExceeded` RuntimeError；
- detached stream task保存 `OwnedExecutionControl`，不只保存 cancellation token；provider item wait和
  publication wait都选择 cancellation/deadline/provider；
- timeout/cancel 都调用 provider request cancel；timeout必须向 consumer发送 terminal execution-control
  error，不能静默归类为 stream cancellation；
- stream terminal carrier可新增 runtime-internal
  `ProviderTerminal::ExecutionControl(RuntimeError)` 等分支，但不改变 artifact wire。

### 6.5 Callback、WebSocket 与 gateway

| 路径 | 删除 | 保留 |
| --- | --- | --- |
| `runtime/eval/src/assembly_execution/callback_native.rs` | executable vs callback operation summary equality | exact target、slot、method ABI、receiver ABI、params/return |
| `runtime/eval/src/assembly_execution/websocket_contract_plan.rs::validate_executable` | executable vs service contract `maySuspend` equality | linked executable存在、request/context/message/result shape |
| `artifact-model/src/websocket_ingress.rs` admission | derived `NotCancellable` requirement | Unary、callbacks none、persistable context、gateway-owned shape |
| `runtime/request/src/http_gateway_target.rs` 与 `runtime/eval/src/runtime_http_gateway.rs` | 无 | concrete linked executable vs Package callable exact summary |

如果未来 HTTP/WS ingress需要独立 external cancellation policy，它属于 gateway entry/deployment owner，
不得把 callee summary或旧 `BoundaryCancellationContract` 放回共享 ServiceContract。本 DAG 的现有
gateway行为不要求新增该 wire。

### 6.6 D 类 direct fixture 全集

所有直接构造旧 operation/cancellation field 的 fixture family：

- model：
  `artifact-model/src/{boundary.rs,ecosystem_authoring.rs,service_contract.rs,websocket_ingress.rs,websocket_ingress/tests.rs}`；
- compiler：
  `compiler/projection/src/package_artifact/{boundary/types.rs,boundary/eligibility.rs}`，
  `compiler/contract/src/{projection.rs,tests.rs}`，
  `compiler/input/src/contract_dependencies/tests.rs`，
  `compiler/source/src/{contract_dependency_test_fixture.rs,expression_type_model/contract_call_typing.rs}`，
  `compiler/lowering/src/source_file_lowering/interface_execution_tests/contract_fixture.rs`，
  `compiler/compiled/tests/public_instance_signature_handoff.rs`，
  `compiler/tests/{compiler_owned_std_type_resolution.rs,service_conformance.rs,file_ir_execution_type_representation.rs,shared_fixture_lane_probes.rs,websocket_ingress.rs}`；
- deployment：
  `deployment/src/{projection/tests.rs,projection/tests/eligibility.rs,storage/tests.rs,assembly/tests/fixtures.rs}`；
- runtime：
  `runtime/eval/src/assembly_execution/{mod.rs,ordinary.rs,ordinary/tests.rs,ordinary/tests/service_error_consumer.rs,ordinary/tests/source_inline_effect_e2e.rs,async_stream_cancel.rs,websocket_contract_plan.rs,boundary_materialization/tests.rs,projection.rs}`，
  `runtime/{linker,loader,host,package-test}/**/*fixtures*` 与 host admission execution tests；
- tooling：
  `scripts/lib/{runtime-execution-boundary-subjects.mjs,runtime-execution-boundary-self-test.mjs}`及
  `scripts/tests/runtime-execution-boundary-checker.test.mjs`。

这些 fixture 中相邻 `CallableMayEffects.may_suspend`、executable/public signature bit必须保留；只删
contract/callback/interface-owned字段。

## 7. F391 E：strict schema 与 identity generation

### 7.1 精确 generation 表

| artifact / identity | 当前 | 终态 | 原因 |
| --- | --- | --- | --- |
| FileIR schema / identity | `skiff-file-ir-v8` | 保持 v8 | executable bit保留，interface FileIR本来无 bit |
| Publication ABI schema / identity | v1 | 保持 v1 | concrete public callable grammar不变 |
| PackageUnit schema | `skiff-package-unit-v1` | `skiff-package-unit-v2` | shared interface DTO删字段 |
| legacy Package local ABI marker/prefix | `skiff-package-local-abi-identity-v2` / `skiff-package-local-abi-v2:sha256` | 两者保持不变 | preimage只含 publication ABI identity + ABI facts，不含 InterfaceMethodSignature |
| Package implementation-links prefix | `skiff-package-implementation-links-v1:sha256` | `skiff-package-implementation-links-v2:sha256` | preimage中的 interface method DTO grammar改变 |
| legacy Package build marker/prefix | `skiff-package-build-identity-v2` / `skiff-package-build-v2:sha256` | `skiff-package-build-identity-v3` / `skiff-package-build-v3:sha256` | build preimage含 implementation links |
| PackageArtifact schema | `skiff-package-artifact-v7` | `skiff-package-artifact-v8` | canonical Local ABI type method DTO删字段 |
| canonical Package local ABI marker/prefix | `skiff-package-artifact-local-abi-identity-v4` / `skiff-package-local-abi-v6:sha256` | `skiff-package-artifact-local-abi-identity-v5` / `skiff-package-local-abi-v7:sha256` | public_symbols grammar改变 |
| canonical Package build marker/prefix | `skiff-package-artifact-build-identity-v6` / `skiff-package-build-v8:sha256` | `skiff-package-artifact-build-identity-v7` / `skiff-package-build-v9:sha256` | local ABI、links、schema refs与boundary projection改变 |
| PackageSchemaType marker/prefix | `skiff-package-schema-type-identity-v1` / `skiff-package-schema-type-v1:sha256` | `skiff-package-schema-type-identity-v2` / `skiff-package-schema-type-v2:sha256` | callback descriptor grammar删 bit；generation全局切换 |
| PackageSchemaIndex marker/prefix | `skiff-package-schema-index-identity-v1` / `skiff-package-schema-index-v1:sha256` | 两者保持不变 | index结构不变；其中 type refs会自然换值 |
| ContractOperation marker/prefix | `skiff-contract-operation-identity-v1` / `skiff-contract-operation-v1:sha256` | 两者保持不变 | 仍只由 service id + stable operation key生成 |
| ServiceContractDefinition schema | `skiff-service-contract-definition-v3` | `skiff-service-contract-definition-v4` | operation body删字段 |
| ServiceContract schema | `skiff-service-contract-v4` | `skiff-service-contract-v5` | operation body删字段 |
| ServiceProtocol marker/prefix | `skiff-service-protocol-identity-v4` / `skiff-service-protocol-v4:sha256` | `skiff-service-protocol-identity-v5` / `skiff-service-protocol-v5:sha256` | protocol preimage grammar改变 |
| ServiceDeploymentInput schema | `skiff-service-deployment-input-v2` | 保持不变 | 统一 lane不加 metadata wire |
| ServiceDeployment schema / identity | `skiff-service-deployment-v2`；`skiff-deployment-artifact-identity-v2` / `skiff-deployment-artifact-v2:sha256` | 三者保持不变 | preimage结构不变；exact refs换值 |
| RuntimeAssembly schema / identity | `skiff-runtime-assembly-v2`；`skiff-runtime-assembly-identity-v2` / `skiff-runtime-assembly-v2:sha256` | 三者保持不变 | preimage结构不变；resolved refs换值 |
| PackageArtifact pointer / schema record path grammar | v1 / 现有 path framing | 保持 | 结构不变；嵌套 build/local/schema identities使用新prefix |

PackageSchemaType generation是整类 generation，不只 callback record。因此即使生态中当前没有
callback-interface record，所有 record的 ID也会换 v2 prefix，PackageSchemaIndex entries、Package
build、引用这些 type的 ServiceProtocol 会传递重算。

### 7.2 Strict rejection owner

- `#[serde(deny_unknown_fields)]` 令新
  `InterfaceMethodSignature`、`BoundaryCallbackOperation`、
  `BoundaryOperationContract` 直接拒绝旧 `maySuspend` / `cancellation`；
- `artifact-model/src/schema.rs` 与
  `artifact-identity::{artifact_reference.rs,package_resolver.rs,contract.rs}` 的 top-level
  validation拒绝旧 PackageUnit/PackageArtifact/ServiceContract generation；
- authoring validation拒绝旧 ServiceContractDefinition generation；
- `artifact-identity/src/ecosystem_paths.rs`、identity lexical parse 和 Router exact regex只接受新prefix；
- 禁止旧字段默认、unknown passthrough、dual-read、hash fallback。

### 7.3 Identity/golden mutation matrix

共享 checkpoint 必须实现 F391 的十格测试，并明确预期：

1. interface同 shape 的 concrete `false`/`true` 均 conform；
2. public signature与 concrete implementation summary不等仍拒绝；
3. concrete public summary mutation：canonical Local ABI/build改变，PackageCallableId稳定；
4. dependency summary `false`/`true` exact传播；
5. interface/unknown target为true但不生成runtime yield；
6. provider summary mutation：ServiceContract canonical bytes、protocol identity、operation ID不变；
   Package build、deployment、assembly改变；
7. service caller面对两种provider summary都为true且走同一runtime lane；
8. callback PackageSchemaTypeId不因implementor summary变化；
9. 新wire拒绝三类legacy字段；
10. request/response/stream/callback shape mutation仍改变 PackageSchemaType/ServiceProtocol；
    stable operation key不变时 ContractOperationId稳定。

`artifact-identity/src/{constants.rs,contract.rs,package.rs,package/projection.rs,package_artifact/projection.rs,tests/mod.rs,tests/package.rs,tests/golden.rs,tests/canonical_compile_contract/package_artifact_identity.rs}`、
`artifact-identity/tests/identity_cli.rs` 与 compiler projection golden是该矩阵的owner。

## 8. Router、scripts 与 cross-system consumer

generation不能只改 Rust constants。精确 tooling write list是：

| consumer | 当前 gate | 终态 |
| --- | --- | --- |
| `router/src/router/runtimeAssemblySnapshot.ts` | service protocol v4 | v5 |
| `router/src/router/runtimeAssemblyDeploymentSnapshot.ts` | service protocol v4 | v5 |
| `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts` | canonical package build v8 | v9；FileIR仍v8 |
| `router/src/artifacts/pointerRecords.ts` | legacy PackageUnit v1 | v2 |
| `router/src/artifacts/types.ts::PackageUnitArtifactPointer` | literal v1 | v2 |
| `router/tests/{compilerGeneratedManifestCompatibility,dynamic-build-id-parity,filesystem-runtime-assembly-snapshot-loader}.test.ts` | current exact fixture | 直接消费新 fresh records，不做replace |
| `router/tests/artifacts.test.ts` legacy pointer fixtures | PackageUnit v1/build v2 | v2/build v3 |
| `scripts/check-artifact-identity-single-source.mjs` | implementation-links v1等 self-fixture | v2及新 canonical prefixes |
| `scripts/tests/artifact-identity-validation.test.mjs` | PackageUnit v1/build v2 | v2/build v3 |
| `scripts/tests/package-service-authoring.test.mjs` | protocol v4 | v5 |
| `cross-system-fixtures/dynamic-build-id-parity/case.json` | old generated identities | 用新 compiler完整再生 |

cross-system fixture中的 FileIR executable、legacy/canonical concrete public signature
`maySuspend` 均属于 B 类，必须保留。不要把 Router 全局出现的 legacy runtime protocol v3 字符串做
机械替换；只有上表 canonical artifact parser属于本 generation切换。

## 9. Current production ecosystem 全量扫描

### 9.1 Interface/concrete pair baseline

“pair”仍指一个 source `implements` method与其 requirement。迁移前 current baseline：

| owner | pairs | concrete `false` | concrete `true` | 证据 |
| --- | ---: | ---: | ---: | --- |
| `skiff.run/std`、http-session、track | 0 | 0 | 0 | source/artifact scan |
| `agine.ai/llm-api`、llm-providers | 0 | 0 | 0 | LlmClient owner无本包implementor |
| `agine.ai/agent` | 73 | 39 | 34 | current source隔离 FileIR，逐个 `implMethod.maySuspend` |
| `agine.ai/codex-relay` | 2 | 0 | 2 | method body与F382相同；只改 implicit receiver spelling |
| `agine.ai/aihub` | 5 | 3 | 2 | 五个impl body与F382相同 |
| `skiff.run/account` | 1 | 1 | 0 | body与F382相同 |
| `agine.ai/api`（Agine） | 17 | 4 | 13 | F382 15（4/11）body未改；新增 AgineSocket connect/receive均到达真实 service/db/ws路径 |
| `skiff.run/registry` | 1 | 1 | 0 | 新 `RegistryServiceImpl.ping` 直接返回true |
| **合计** | **99** | **48** | **51** | current source baseline |

主统计继续排除 `*.test.skiff`，纳入不以该后缀结尾的 `*_test_support.skiff`。

这是迁移前事实，不是对 F391 编译器输出的预言。dependency Package call从“总是true”改成 exact
summary 后，部分 concrete executable可能从true收窄为false；所有99 pair都要在新 compiler fresh
重算，不能把 48/51 写成新 golden。Relay两方法自身仍到达真实等待，必须保持true。

F382记录的 mixed implementation operation仍存在，包括
`LlmClient.webSearch`、`AgentEventReceiver.receive`、`SubagentDelegate`三方法、
`ToolProvider`三方法、Drain checkpoint methods、`DrainToolPort.execute` 与
`PendingUserMessageProbe`。这正是 interface不得拥有 exact bit的生产证据。

### 9.2 Callback schema scan

current source-visible interface owners包括：

- llm-api `LlmClient`；
- Agent 的 event/subagent/tool/drain/cleanup/queued-domain interfaces；
- Relay `CodexRelayProxyClient`；
- Aihub `AihubManagedLlmClient` / `AihubProviderCatalog`；
- Account `AccountService`；
- Agine `AgineSocket`；
- Registry `RegistryService`。

隔离 canonical records已证明 std、http-session、llm-api、llm-providers、Agent 的156个 schema
records和F382 Relay均无 callback-interface record。Relay/Aihub/Account 的 public instance receiver
不是 callback capability；AgineSocket属于 WebSocket ingress interface。

但 current service-only Aihub/Agine/Account/Registry无法由 phase-05 compiler诚实发 fresh records，
所以本审计不把“全生态最终仍为0”伪装成receipt。`N5`必须遍历每个新 PackageArtifact 的
PackageSchemaType records，列出 callback-interface count与stable key；若出现 record，断言其 operation
无 `maySuspend` 且 implementor summary mutation不改变 type ID。

### 9.3 ServiceContract scan

current production service roots共五个：

| service | source-visible suspension相关 surface | 当前可给出的精确事实 |
| --- | --- | --- |
| Relay | `relayProxy.responsesCompleted`、`responsesCompletedResult` | F382旧 authoring exact为2 operations；current source仍有同两方法 |
| Aihub | managedLlm 3 methods、providerCatalog 2 methods；HTTP/WS gateway | public instance pair共5；实际新 contract operation set须由 integrated authoring receipt决定 |
| Account | accountService `ping`；21条HTTP routes | interface pair 1；gateway routes不应被误当callee summary protocol |
| Agine | AgineSocket connect/receive；HTTP gateway | WebSocket ingress shape；不从gateway executable summary生成ServiceContract bit |
| Registry | registryService `ping`；public/org/build HTTP routes | interface pair 1；实际新 contract operation set须 fresh读取 |

旧 `serviceCall` marker在 current Relay已经消失，所以除 Relay稳定的两条 method key外，本审计不从
api.yml 文本臆造新 authoring的 operation count。无论实际 operation set为何，每条新
ServiceContract v5 operation body都不得含 `maySuspend`/`cancellation`。

### 9.4 受影响 artifact 与 rebuild 顺序

generation是 strict全局切换，因此即使 source没有interface/callback：

- 所有 PackageUnit v2 / PackageArtifact v8重新生成；
- 所有 canonical Local ABI v7、build v9、PackageSchemaType v2、PackageSchemaIndex refs重新生成；
- 所有 ServiceContractDefinition v4、ServiceContract v5、ServiceProtocol v5重新生成；
- 所有 ServiceDeployment/RuntimeAssembly保持v2结构但重算 exact refs与 identity values；
- FileIR只在 effect fixed point结果改变时换identity；不能仅因 generation无条件改写。

最小 real rebuild：

```text
std
  -> llm-api
  -> llm-providers
  -> Relay  (首个真实 2-operation proof)
  -> { Agent, http-session, track, Aihub, Account, Registry }
  -> Agine
```

Account等待http-session；Aihub等待Relay；Agine等待Aihub、Agent、http-session、track与llm packages。
可在 Relay proof后按依赖并行其余节点。

Relay proof必须断言：

- operation set精确只有
  `relayProxy.responsesCompleted` 与 `relayProxy.responsesCompletedResult`；
- 两个 concrete PackageCallableSignature均为 `maySuspend=true`；
- interface method wire没有 `maySuspend`；
- ServiceContract operation body没有 `maySuspend`或`cancellation`；
- operation ID仍为：
  - `skiff-contract-operation-v1:sha256:b62d89d553cc0607b2627b047d2a5ab4665c70f05f900babbce249def47099ef`
  - `skiff-contract-operation-v1:sha256:51fa082dd0d33b09f45e4900805c28801cb3108b4eac813697e66e5f8a6b007d`
- receipt generation为 PackageArtifact v8 / Local ABI v7 / build v9 /
  ServiceContract v5 / protocol v5 / deployment与assembly v2；
- 不复用旧store、old lock、F382 protocol或validator waiver。

## 10. 唯一最小无重叠实现 DAG

### 10.1 图与并发上限

```text
G0 integrated-source gate (write set ∅)
  |
N0 schema + identity checkpoint
  |\
  | +-----------------+
  v                   v
N1 compiler        N2 deployment        N3 runtime
  \                   |                 /
   +------------------+----------------+
                      v
              N4 Router + tooling
                      |
                      v
              N5 fresh ecosystem proof
```

`N1`、`N2`、`N3` 在 `N0` 后最多使用3个worker并行。所有节点 production write set两两不交叠；
fixture随其owner目录走，不建立共享“tests节点”。

### 10.2 G0 — integrated source gate

- production write set：`∅`；只验证候选实现base。
- dependency：phase-05 tree `a3bd663...` 与 current service-only authoring都已进入同一 descendant
  commit。
- 正测试：candidate compiler既认识 PackageArtifact v7/ServiceContract v4 baseline fields，又能读取
  current Relay只有 `service.yml` 的root。
- 负测试：缺任一侧时立即停止；不得临时恢复 retired production `package.yml`/`serviceCall` 来制造
  receipt。
- focused commands：
  `git merge-base --is-ancestor <required-commit> HEAD`、schema constant反搜、对 current Relay执行
  authoring dry-run到 fresh `/tmp` store。
- receipt：记录 candidate commit/tree、dry-run命令、root classification与失败前后没有stable write。
- scope expansion：若 current service-only authoring没有等价的 PackageArtifact + ServiceContract
  canonical output，先开独立 authoring reconciliation节点；不在 suspension patch猜新格式。

### 10.3 N0 — shared schema/identity checkpoint

- production write set：`artifact-model/**`、`artifact-identity/**`。
- dependency：G0。
- 内容：A/D字段删除、E generation/constants/preimages/path validation、strict serde、mutation/golden。
- 正测试：十格矩阵；new generation round-trip；shape mutation改变protocol；provider summary mutation不改
  contract/protocol。
- 负测试：legacy interface/callback/service fields、旧top-level generation、上表所有被提升的旧prefix
  全部拒绝；
  public/concrete summary mismatch仍拒绝。
- focused commands：

  ```bash
  cargo test --manifest-path artifact-model/Cargo.toml
  cargo test --manifest-path artifact-identity/Cargo.toml
  cargo test --manifest-path artifact-identity/Cargo.toml --test identity_cli
  ```

- identity receipt：输出每个新 marker/prefix、old-wire rejection、mutation前后 Local ABI/build/
  PackageSchemaType/ServiceProtocol/operation/deployment/assembly identity矩阵。
- scope expansion：发现任何新增 public wire承载provider summary，或必须兼容旧generation时停止；
  F391禁止兼容读取。

### 10.4 N1 — compiler inference/projection

- production write set：`compiler/**`。
- dependency：N0。
- 内容：A interface/callback producer清理；B concrete handoff保持；C explicit target matrix；
  D code-free contract projection；所有compiler-owned fixture。
- 正测试：两种implementation均conform；public/concrete exact handoff；dependency false/true；interface、
  service、unknown true；callback/schema无bit。
- 负测试：missing dependency summary fail closed；public implementation mismatch拒绝；test-effect不能伪造
  provider bit；旧 contract fixture反序列化失败。
- focused commands：

  ```bash
  cargo test --manifest-path compiler/core/Cargo.toml package_interface
  cargo test --manifest-path compiler/source/Cargo.toml callable_effects
  cargo test --manifest-path compiler/lowering/Cargo.toml suspend
  cargo test --manifest-path compiler/projection/Cargo.toml package_artifact
  cargo test --manifest-path compiler/compiled/Cargo.toml
  cargo test --manifest-path compiler/contract/Cargo.toml
  cargo test --manifest-path compiler/Cargo.toml --test service_conformance
  cargo test --manifest-path compiler/Cargo.toml --test file_ir_execution_type_representation
  ```

- receipt：序列化 call-target/effect matrix；两种 concrete summary 的 PackageArtifact refs；证明
  ServiceContract bytes相同；FileIR无 interface bit或synthetic yield。
- scope expansion：如果 exact dependency summary在 canonical PackageArtifact中不可达，先修复已有
  dependency ingest；不得读取dependency source或在interface加bit。

### 10.5 N2 — deployment admission

- production write set：`deployment/**`。
- dependency：N0；与N1/N3并行，最终集成验收读取N1 artifact。
- 内容：删除 contract summary/cancellation equality；保留 exact semantic facts /
  implementation requirements / shape / provenance admission。
- 正测试：provider summary false/true在相同 code-free contract下均可绑定；summary mutation令 exact
  PackageArtifactRef、deployment、assembly value改变。
- 负测试：unknown effects、complete may-effects mismatch、provenance mismatch、operation shape mismatch
  继续失败。
- focused commands：

  ```bash
  cargo test --manifest-path deployment/Cargo.toml projection
  cargo test --manifest-path deployment/Cargo.toml storage
  cargo test --manifest-path deployment/Cargo.toml assembly
  ```

- identity receipt：同一 ServiceContractRef + 两个provider build refs的deployment/assembly diff；
  schema/prefix仍v2。
- scope expansion：若runtime设计要求在deployment内联summary，停止并另做Deployment/Assembly v3
  schema审计；本节点不得自行加字段。

### 10.6 N3 — runtime unified wait/cancellation

- production write set：`runtime/**`。
- dependency：N0；与N1/N2并行。
- 内容：统一service lane、删除ordinary service path、deadline API/owned control、stream terminal error、
  callback/WS validator迁移；concrete package/gateway checks保留。
- 正测试：
  Ready unary同poll完成；Pending后完成；ancestor/request cancel；deadline；cancel/deadline同时ready优先
  cancel；provider typed error materialization；stream item/publication cancel与deadline；callback/WS
  summary不同仍按shape接受。
- 负测试：
  shape/target/ABI mismatch仍拒绝；deadline不能变Cancelled；timeout后provider request必须cancel；
  stream task/lease计数归零；Unsupported stream/callback仍typed失败。
- focused commands：

  ```bash
  cargo test --manifest-path runtime/capability-context/Cargo.toml execution_control
  cargo test --manifest-path runtime/request/Cargo.toml execution_budget
  cargo test --manifest-path runtime/model/Cargo.toml callback_projection
  cargo test --manifest-path runtime/eval/Cargo.toml assembly_execution
  cargo test --manifest-path runtime/native/Cargo.toml callback_adapter
  cargo test --manifest-path runtime/linker/Cargo.toml assembly
  cargo test --manifest-path runtime/loader/Cargo.toml runtime_assembly
  cargo test --manifest-path runtime/host/Cargo.toml assembly_admission
  ```

- runtime receipt：Ready poll计数、Pending wake来源、typed deadline/cancel结果、provider cancel signal、
  active stream task/lease归零，以及两种provider summary执行trace等价。
- scope expansion：只有现有 cancellation token不是ancestor/request composed时才扩到activation owner；
  仍不得回填ServiceContract cancellation。

### 10.7 N4 — Router/tooling/golden

- production write set：`router/**`、`scripts/**`、`cross-system-fixtures/**`。
- dependency：N1、N2、N3全部通过。
- 内容：第8节精确parser/generation更新、boundary checker清理、fresh dynamic fixture再生。
- 正测试：Router direct join与filesystem loader直接读取 exact fresh records；identity single-source
  checker通过；legacy PackageUnit pointer的新正例通过。
- 负测试：v4 protocol、v8 canonical build、v1 PackageUnit、v2 legacy build以及old fields全部拒绝；
  path escape/duplicate key负例保留。
- focused commands：

  ```bash
  node --test \
    scripts/tests/artifact-identity-validation.test.mjs \
    scripts/tests/runtime-execution-boundary-checker.test.mjs \
    scripts/tests/package-service-authoring.test.mjs
  node scripts/check-artifact-identity-single-source.mjs
  pnpm --filter @skiff/router exec vitest run \
    tests/compilerGeneratedManifestCompatibility.test.ts \
    tests/dynamic-build-id-parity.test.ts \
    tests/filesystem-runtime-assembly-snapshot-loader.test.ts
  pnpm --filter @skiff/router exec tsc --noEmit --pretty false
  ```

- receipt：fresh fixture compiler commit/tree、record path、schema/prefix集合、Router test count、旧generation
  反搜零匹配。
- scope expansion：若另一个 canonical Router loader消费旧prefix，加入本节点write set并证明无重叠；
  不改独立legacy runtime protocol domain。

### 10.8 N5 — fresh ecosystem与Relay-first proof

- production write set：`∅`；只写 fresh `/tmp` source mirror/artifact store/receipts。若必须改 ecosystem
  source，另开显式migration节点。
- dependency：N4。
- 正测试：第9.4节完整顺序；Relay先满足全部2-operation assertions，之后才验Aihub/Account/
  Registry/Agine ServiceContracts；重算99 pairs和callback records。
- 负测试：任一旧prefix/字段、operation count非2、concrete Relay summary非true、operation ID变化、
  consumer引用旧protocol均失败。
- focused command骨架：

  ```bash
  probe_root="$(mktemp -d /tmp/p5-f395-acceptance.XXXXXX)"
  export CARGO_TARGET_DIR="$probe_root/cargo-target"

  cargo run --quiet --manifest-path test-runner/Cargo.toml \
    --bin skiff-package-service-smoke-fixture -- \
    --bootstrap-only \
    --artifact-root "$probe_root/artifacts" \
    --environment dev \
    --platform-source-root "$PWD"

  cargo run --quiet --manifest-path compiler/Cargo.toml \
    --bin skiff-compiler -- package publish <llm-api-root> \
    --artifact-root "$probe_root/artifacts" --environment dev \
    --platform-source-root "$PWD" --json
  cargo run --quiet --manifest-path compiler/Cargo.toml \
    --bin skiff-compiler -- package publish <llm-providers-root> \
    --artifact-root "$probe_root/artifacts" --environment dev \
    --platform-source-root "$PWD" --json

  # G0选定的current service-only canonical authoring命令：
  <publish-current-relay-package-and-contract-to-probe-root>
  <build-current-relay-deployment-and-assembly-to-probe-root>
  ```

  最后两条必须由G0 receipt给出实际命令，不能用本文当前不兼容的 phase-05 CLI冒充。

- receipt：每步 input commit/tree、command、stdout JSON、PackageArtifact/Contract/Deployment/Assembly
  record paths与SHA-256、schema/prefix、operation列表、pair分类、callback列表、consumer exact refs。
- scope expansion：service-only tooling若不能发 PackageArtifact/ServiceContract，回到G0独立解决；
  Relay若不是exact两操作则停止调查authoring差异，不放宽validator。

## 11. 最终验收判定

F391没有内部矛盾。唯一 implementation boundary是：

- N0原子切 strict schema/identity；
- N1/N2/N3按目录并行且不共享write set；
- N4统一收口生成物consumer；
- N5用current service-only integrated source做Relay首个真实2-operation proof，再重建全生态。

实现完成前不能宣称 Relay fixed；尤其不能使用 `{}` body、旧protocol、old lock、字段默认、validator
waiver或stable instance输出替代 N5 receipt。
