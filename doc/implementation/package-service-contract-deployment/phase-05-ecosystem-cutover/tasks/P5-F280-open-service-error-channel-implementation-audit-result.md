# P5-F280 Open service error channel implementation audit result

状态：Audit complete；解除共享 artifact/language checkpoint、runtime/wire checkpoint 与生态迁移任务。

## 审计结论

当前实现不是“已有 typed error channel 只差放宽”，而是三套彼此不兼容的旧形状叠在一起：

1. source checker 不执行权威 `CatchLeaves` 规则，几乎任意表达式都能进入 `throw` / `rethrow`，
   任意可解析类型都能进入 `catch<E>`；名义 record 只是在后续 runtime 地址匹配中偶然正确。
2. File IR 把 representation 与透明 alias 都降成 `Alias`，把 interface 降成空 `Record`，named union
   也没有 concrete / synthetic / literal branch identity 或 enclosing union context；throw、call
   instruction 又没有自己的 source span。因此 representation、named union、非法 catch leaf 与完整异常栈
   都不能仅靠修改 eval 恢复。
3. Package ABI 与 ServiceContract 仍携带 operation-specific closed throw set，但 production source producer
   永远生成空集合。canonical in-process boundary 因而在第一个 service boundary 把所有
   `UserException`改成`Protocol`错误；现有 typed materialization 分支只由手写 fixture 才能到达。

现有 Package public schema、fresh provider heap、directional boundary codec、exact package type export 与
runtime diagnostic wrapper 是可复用底座，但缺少四个 canonical owner：

- 可区分 declaration kind、union branch/context 且带 instruction source span 的 language/File IR identity；
- 从 runtime `TypeAddr` 双向定位 owner Package public schema identity 的 assembly-owned index；
- 唯一的固定 `ServiceErrorEnvelope` 与 boundary export/import/opaque-forward orchestrator；
- request-scoped `traceId/errorId`、本地异常栈与受限 telemetry error event owner。

不应保留旧 field、旧 wire、dual read/write、router 侧类型分类或按 display/shape 猜类型。Skiff 尚未发布，
可以一次严格切换。

本审计基于 integration checkpoint `512135dd`，当前审计基线为`f936fd0b`。除本文外没有修改 production、
fixture、std、reference、F278 写入面或其它仓库，也没有运行 build、test、stable instance、live 或发布命令。

语义判定只使用直接父结果`P5-F279-open-service-error-channel-design-result.md`及其引用的
`doc/architecture/package-service-contract-deployment.md` §6.3、
`doc/reference/static-semantics.md` §5/§16、`doc/reference/runtime.md` §7、
`doc/reference/std-surface.md` §3/§4与`doc/reference/observability.md` Event Shape。F274仅用于识别旧实现
残留，不作为设计依据。

## 一、production 链：真实顺序、当前形状与首次语义损失

### 1. 编译、投影与 admission 顺序

| 顺序 | production 跳点 | 当前形状 | 首次语义损失 | 文件与 symbol 证据 |
| --- | --- | --- | --- | --- |
| 1 | parser 读取 type / throw / catch | named union 的 anonymous record branch 会检查唯一 string literal discriminator；throw/catch 语法本身不限制 payload | parser 已掌握 discriminator，却不产出 synthetic branch identity | `syntax/src/parser.rs::validate_type_decl_discriminator` |
| 2 | source interface index | compiler 无源码地注入一个零 method 的`std.error.ErrorPayload` interface，故`implements ErrorPayload`只是一项普通空 marker conformance | marker 没有参与 throw/catch 合法性，现有声明产生了“必须实现 marker”的假象但没有语义作用 | `compiler/source/src/semantic/interface.rs::{InterfaceIndex::build,insert_compiler_known_interface}` |
| 3 | source expression typing | statement/expression throw 只调用`check_expr`；rethrow 也只检查表达式；catch 只解析类型并构造`CatchResult` | **非法 primitive/interface/unknown/function/unconstrained generic throw/catch 与非`Exception<E>` rethrow 在这里首次未被拒绝** | `compiler/source/src/expression_type_model.rs::{check_stmt,check_expr}`中的`Stmt::Throw`、`Stmt::Rethrow`、`Expr::Throw`、`Expr::Rethrow`、`Expr::Catch` |
| 4 | File IR lowering | throw 只保存 static `payload_type`；`TypeDeclIr`只有`Record/Alias/Union` descriptor；representation 与 transparent alias 同形，interface 是空 record | **declaration kind、named-union enclosing context、synthetic/literal branch identity 在这里首次不可逆丢失** | `compiler/lowering/src/{declaration_lowering.rs::lower_type_declarations,function_lowering.rs::throw_payload_type}`；`artifact-model/src/{types.rs::TypeDeclIr,executable.rs::{StmtIr,ExprIr}}` |
| 5 | source map lowering | declaration/function/db 节点会写 source span；`StmtIr::Throw`、`ExprIr::Throw`与`CallIr`没有 span | **throw site 与 service call site 在这里首次不可逆丢失** | `compiler/lowering/src/{source_unit_lowering.rs,executable_declaration_lowering.rs,declaration_lowering.rs,db_lowering.rs}`；`artifact-model/src/executable.rs::{ExecutableIr,CallIr,StmtIr,ExprIr}` |
| 6 | Package callable signature producer | source public callable 与 implementation-only callable 都写`throw_types: []`，语言没有 declared throws producer | production artifact 的 closed throw set 恒为空；非空只来自手写测试/依赖 artifact | `compiler/source/src/contract_type_resolution.rs::SourceExecutableSignature::package_callable_signature`；`compiler/projection/src/package_artifact/callables/mod.rs::project_implementation_symbols` |
| 7 | Package ABI handoff/projection | signature 经 compiled/projection-input handoff、normalization、dependency identity binding；boundary projection把 0/1/N 个 throw type变成`None/Typed/structural union` | 空集合被提升成 canonical ABI/contract 事实，而不是被识别为旧模型残留 | `compiler/compiled/src/package_callable_signatures.rs::build_package_callable_signatures`；`compiler/projection-input/src/package_callable_signatures.rs::ProjectionPackageCallableSignatureFacts`；`compiler/projection/src/package_artifact/{callables/normalization.rs::normalize_public_signature,boundary/types.rs::project_operation_contract}`；`compiler/driver/source_compile/canonical_dependencies.rs::bind_callable_signature_identity` |
| 8 | validation 与 identity | public throw types被验证；整个 public symbol进入 Package Local ABI preimage；整个 operation 进入 ServiceProtocolIdentity preimage | 即使 production 值恒为`[]`/`None`，field 的存在仍改变 Local ABI 与 protocol identity schema | `artifact-identity/src/package_artifact/{validation.rs::validate_callable_surfaces,projection.rs::local_abi_projection}`；`artifact-identity/src/contract.rs::{ServiceProtocolIdentityProjection,service_protocol_identity_projection}` |
| 9 | contract/schema closure | error payload参与 contract normalization、existential validation、Package schema roots、deployment eligibility、loader/test-runner closure | operation throw set继续决定 admission schema，而开放错误真正需要的是 owner Package 自己的 public schema | `artifact-identity/src/contract/normalization.rs::normalize_contract_operation_contract`；`artifact-identity/src/contract.rs::{validate_operation_existentials,collect_operation_refs}`；`compiler/contract/src/projection.rs::collect_operation_refs`；`deployment/src/projection/eligibility.rs::validate_contract_features`；`runtime/loader/src/runtime_assembly.rs::load_contracts`；`test-runner/src/package_schema_contract.rs::collect_operation_refs` |
| 10 | Package schema admission | api.yml 显式 public、非透明 alias、`SchemaClosed`的类型已经获得`packageId/stableSchemaKey/PackageSchemaTypeId` index/record；PackageArtifact只嵌 index ref与record refs，loader目前只解析 record closure | runtime既没有解析 exact PackageSchemaIndex，也没有从执行时`TypeAddr`反查 public record、或从 envelope type id反查 linked local identity 的 assembly index | `compiler/projection/src/package_artifact/schema.rs::{project_package_schema,SchemaBuilder::build}`；`artifact-model/src/{package_artifact.rs::PackageArtifact,contract_types.rs::{PackageSchemaIndex,PackageSchemaIndexRef}}`；`runtime/loader/src/runtime_assembly.rs::{RuntimeAssemblyContentResolver,load_packages,load_package_schema_closure}`；`runtime/linked-program/src/shared_image.rs::SharedPackageCode::schema_records` |

### 2. 当前各类 throw/catch 的实际结果

| 类型形状 | source / File IR 当前结果 | runtime 当前结果 | 与权威语义的差异 |
| --- | --- | --- | --- |
| 名义 record | `TypeDeclIr::Record`，throw static address | throw identity 与 catch leaf 都是同一`TypeAddr`，本地 exact catch 可工作 | 值 identity 尚未显式附着在`RuntimeValue`上，且 source/stack 仍为空 |
| 名义 representation `type R = RHS` | 与透明 alias 一样降成`TypeDescriptorIr::Alias` | throw address取外层`R`；catch 却展开 RHS，primitive RHS 会得到零 leaf | representation 应保留外层 nominal identity，不能按 alias 展开 |
| 透明 `alias A = RHS` | 同样是`Alias` | catch 展开 RHS | 展开方向正确，但 runtime 无法证明它是 alias 而非 representation |
| named union，concrete nominal branch | union declaration只保存 variants | static payload 为 union 时 throw记录 enclosing union address，catch union展开为 branch addresses，两者不匹配；static payload 为 branch 时又没有 enclosing context | 应记录实际 concrete branch identity并保留 named-union context |
| named union，anonymous discriminator branch | parser验证 tag，IR只剩 anonymous record shape | anonymous record不进入 catch leaves，没有 stable synthetic branch id | 应由全限定 union id、完整 type args 与 discriminator value派生 synthetic id |
| named union，literal branch | IR保留 literal shape | literal不进入 catch leaves | 应由 enclosing union id 与 literal payload派生 branch id |
| anonymous union `A | B`，两边均为 nominal | source checker接受 | catch可收集两个 address leaf；throw static union却因“不是恰好一个 leaf”在 runtime Decode | throw应从实际值读取 A或B identity；anonymous union本身不新增 identity |
| mixed union/nullable，例如`A | string`或`A?` | source checker接受 | runtime忽略非名义 branch；只剩一个 A时不仅允许 catch，throw还可能把实际 string/null错误标成 A | 权威规则要求 union每个可能 branch都有可确定 identity；不能静默丢弃或伪造 branch identity |
| primitive、anonymous record、container、`unknown`、function、无约束 type parameter | source checker接受 | `catch_type_leaves`得到零 leaf，执行时才报 Decode；不是 compile diagnostic | 必须在 source semantic phase 失败关闭 |
| interface | lowering成空`Record` declaration | 可能被 runtime 当作 record address leaf，而 source 没有先拒绝 | interface 永远不是 catch leaf，不能靠空 record shape猜测 |
| generic nominal type | source model允许 type parameter | File IR emitter目前直接拒绝 generic local type usage | 权威 synthetic identity需要完全实例化 type args；需随 identity checkpoint补齐 |

证据：

- `runtime/eval/src/exceptions.rs::{throw_payload_actual_type,catch_type_leaves,collect_catch_type_leaves}`
  对 address record建 leaf、对 address alias展开、对 address union展开、忽略 anonymous record/literal/native
  非标准类型。
- source parser始终要求`catch<E>`，lowerer也始终写`Some(catch_type)`；但
  `artifact-model/src/executable.rs::ExprIr::Catch.catch_type`仍是 optional，且
  `runtime/eval/src/eval_context.rs::eval_program_catch`把`None`变成空 leaves，
  `exceptions.rs::catch_identity_matches`又令空 leaves匹配任意`UserException`。这是只可由手写/旧 IR触达的
  隐式 catch-all，
  会破坏“未链接 middle service不能 catch opaque error”，应在 strict File IR checkpoint删除。
- `artifact-model/src/types.rs::{TypeDeclIr,TypeDescriptorIr}`与
  `runtime/linked-program/src/linked.rs::{TypeDeclIr,LinkedTypeDescriptor}`都不保存 declaration kind。
- `artifact-model/src/package_artifact.rs::PackageLocalAbiSymbol::Type`其实已有`is_alias/is_interface`，
  证明 compiler producer掌握这两个事实；问题是 File IR/linked execution没有同一个 canonical fact，runtime
  不能从 ABI旁路猜 declaration kind。
- `compiler/lowering/src/declaration_lowering.rs::{lower_type_declarations,lower_type_decl_descriptor}`分别把
  representation与透明 alias降成`Alias`、把 interface降成空`Record`。
- `compiler/lowering/src/type_lowering.rs::lower_type_ref`仍报
  `generic local type ... is not supported by the File IR unit emitter yet`。
- `runtime/model/src/error.rs::TypeIdentity`当前只有 execution `Address`与 hard-coded `Builtin`两类，没有
  Package schema identity、named-union context或 synthetic/literal branch id。
- `runtime/model/src/value.rs::RuntimeValue`只携带 primitive/heap handle等值，不携带 nominal/union branch
  identity；`runtime/model/src/type_plan.rs::RuntimeTypeIdentityPlan`虽有
  `nominal/union/union_branch`，但它属于 codec plan，不附着在执行值上。

### 3. request 执行与 service error 的真实顺序

1. Router ingress 已创建 trace。

   `router/src/router/assemblyHttpGateway.ts`与
   `router/src/gateway/assemblyWebSocketGateway.ts`创建`traceId/spanId`；
   `runtime/request/src/context.rs::request_trace_id`把 traceId送进 native
   `InvocationContext`。但这还没有进入`UserException`或普通 assembly request telemetry context。

2. Provider 在 fresh heap 中执行。

   canonical service call 顺序是
   `runtime/eval/src/assembly_execution/mod.rs::dispatch_service_call`
   →`dispatch_in_process_boundary`
   →`ordinary.rs::execute_service_call`或`async_stream_cancel.rs::execute_service_call`
   →`CanonicalServiceBoundaryPlan::new`
   →materialize caller args到 fresh provider heap
   →切 provider activation
   →执行 provider
   →`materialize_provider_result`。

   top-level ingress 也经
   `runtime/eval/src/assembly_execution/ingress.rs::dispatch_ingress_via_in_process_boundary`
   进入同一个`dispatch_in_process_boundary`，不是例外路径。fresh heap与directional codec应保留。

3. 初始 throw 创建 request-local `UserException`，但会过早序列化且 envelope 是残缺的。

   `runtime/eval/src/eval_context.rs::eval_program_throw`先用无 schema 的`runtime_to_wire`转 payload，再由
   `runtime/eval/src/error.rs::UserException::from_typed_payload`写内部 marker/debug identity、
   `"error"`、`"source": null`与`"stack": []`。因此：

   - package-local throw在 catch之前就被迫经过 generic wire conversion，含 interface/capability/其它
     non-`SchemaClosed`字段的合法本地 nominal error可能提前失败；
   - 即使名义 record本地 catch成功，**第一次 throw已经没有权威 source location/stack**。

   这违反“可抛出与可序列化独立”。本地`Exception<E>`必须持有 request-local runtime value/handle（或等价
   exception arena carrier），只有 boundary export才能按 owner schema尝试编码。

4. 本地 catch 与 rethrow。

   `runtime/eval/src/exceptions.rs::exception_envelope_for_catch`按 exact `TypeIdentity`匹配；
   `exceptions.rs::catch_err`再用`runtime_from_wire`把 JSON exception放回 heap；
   `runtime/eval/src/program_execution.rs::eval_program_rethrow_slot`用
   `runtime_to_wire + UserException::from_envelope`重建同一个现有 JSON envelope。当前“rethrow不新建
   semantic envelope”这一点可复用，但 eager wire round-trip不能保留；`Exception<E>`是 request-local
   控制流值，不是 boundary JSON。platform error被 catch projection转换成`UserException`时也会走
   `from_typed_payload`，同样得到空 source/stack。

5. 第一个 service boundary 是当前 typed error 的确定性首次丢失点。

   `runtime/eval/src/assembly_execution/boundary_materialization.rs::CanonicalServiceBoundaryPlan`
   只从`operation.contract.errors`构建一个可选`error_plan`。production contract恒为`None`，所以
   `materialize_provider_error`遇到任何`UserException`都会返回：

   ```text
   Protocol:
     provider threw a typed business error but the contract declares no typed error
   ```

   手写`Typed` contract时，它只读取 envelope 的`"error"`字段，按唯一 declared plan detach payload，
   然后`replace_user_exception_preserving_diagnostics`保留原 TypeIdentity 和 diagnostic wrappers。
   这既没有 owner Package identity、fixed envelope、traceId/errorId，也把 callee diagnostic wrapper带给
   caller，违反“每跳新 exception/新栈、callee完整帧只进本地 telemetry”。

6. 未处理传播当前不是透明传播。

   A 的用户错误在 A→B 第一次 boundary 已经变成`RuntimeError::Protocol`；B无法看到或转发原
   encoded payload。未来 B 未链接 error owner时必须把固定 envelope作为 opaque cause保存在本地
   exception中，catch不匹配但 outward export原样复用 envelope；不能 decode/re-encode，也不能生成新
   errorId。

7. 顶层 request error 再被压平成 generic response。

   canonical ingress通常已经在第 5 步变成 Protocol。若其它 eval entry让`UserException`到达
   `runtime/eval/src/error.rs::user_exception_payload`，它会变成`UnhandledServiceError`，message包含
   actual type display，details包含`actualPayloadType`，可进一步泄露 identity/message。
   `runtime/request/src/assembly_ingress.rs::execute_runtime_assembly_request`把 eval error包装成
   `RequestError`，`runtime/host/src/host/request_entry/assembly.rs`再发`ResponseEvent::Error`。

8. runtime↔router wire只有 generic error header。

   `runtime/transport/src/protocol.rs::{RuntimeErrorFramePayload,ResponseErrorFrameHeader}`只有
   `code/message/status/details`；`runtime/transport/src/response_mapper.rs::response_event_into_frame`
   对`response.error`总是发送空 binary payload。Router 的
   `router/src/protocol/envelope.ts::RuntimeErrorPayload`与
   `router/src/protocol/runtimeProtocol.ts::validateErrorPayload`镜像同一形状，
   `router/src/router/runtimeDispatcher.ts`转成`RuntimeResponseError`。

   `router/src/router/errors.ts::{RuntimeResponseError,runtimeErrorHttpDetail}`只在最终 HTTP 5xx 响应时隐藏
   detail；这不能补救 runtime WebSocket 上已经发送的 source frame或私有字段。

9. legacy outbound path还有第二个不同的损失点。

   `runtime/eval/src/service_dispatch.rs::outbound_router_response_into_result`把
   `OutboundResponse::Error(ResponseError)`一律改成`ProviderUnavailable`并只保留 message。
   canonical assembly production已经要求 activation-relative in-process path；未来 RemoteBoundary应复用
   同一 error channel owner，不能把该 legacy mapper升级成第二套分类器。无法迁移的 legacy fixture/path应
   删除或明确隔离。

### 4. scenario-specific 首次损失汇总

- 非法 primitive/interface/unknown/function/generic catch/throw：source type checking 首次未失败关闭。
- representation、named union、synthetic/literal branch：File IR declaration/branch identity lowering首次丢失。
- 含 non-`SchemaClosed`字段的合法本地 nominal throw：`eval_program_throw::runtime_to_wire`首次错误地要求
  序列化。
- 任意合法本地 nominal throw 的 source/stack：`UserException::from_typed_payload`首次写成 null/empty。
- 任意 production跨 service用户错误：第一跳
  `CanonicalServiceBoundaryPlan::materialize_provider_error`首次变成 Protocol。
- 仍到达 generic request mapper 的用户错误：`user_exception_payload`首次变成
  `UnhandledServiceError`并暴露 display identity。
- legacy router outbound error：`outbound_router_response_into_result`首次变成
  `ProviderUnavailable`。

## 二、语言与 std owner 审计

### 1. `ErrorPayload`真实 owner 与残留

`ErrorPayload`不是 throw gate。它只有以下 production作用：

- `compiler/core/src/prelude_registry.rs::COMPILER_BUILTIN_TYPES`把 bare
  `ErrorPayload`登记为`std.error.ErrorPayload`；
- `compiler/source/src/semantic/interface.rs::InterfaceIndex::build`注入 compiler-known空 interface，
  使 source conformance检查通过；
- std/prelude source用`implements ErrorPayload`标注既有 platform error；
- `scripts/check-skiff-source-layout.mjs`把它当 required compiler builtin；
- `vscode/syntaxes/skiff.tmLanguage.json`与`vscode/scripts/test-grammar.mjs`把它当特殊 type token/示例。

`prelude/error.skiff`当前是 0 bytes，所以没有 source declaration提供额外语义。仓库内真正需要迁移的
Skiff source是：

- `prelude/config.skiff`
- `std/{bytes,db,file,http,json,number,resource,service,time}.skiff`
- `compiler/source/src/contract_type_resolution/tests/interface_signatures.rs`
- `compiler/tests/package_std_schema.rs`
- `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs`
- `runtime/live-tests/internal/db_live.live.test.skiff`

test harness另有 native stub：

- `test-runner/src/canonical_package/tests.rs`
- `test-runner/src/canonical_package/tests/combined.rs`
- `compiler/driver/authoring/{tests.rs,package_publication/tests.rs}`
- `compiler/driver/pipeline/tests/p5_f18a.rs`
- `compiler/input/src/{platform_sources/tests.rs,package_sources/tests.rs}`
- `compiler/source/src/prelude_registry/tests.rs`

后六处同样是`native type ErrorPayload` fixture字符串。

这些是 production/fixture/tooling迁移，不包括`doc/implementation/**`里的历史文字。历史 F274 等记录必须
保留原貌，不能全仓搜索替换。

### 2. `std.service.InternalError`现状

当前有三种名称相似但语义不同的东西：

1. `compiler/core/src/prelude_registry.rs::COMPILER_BUILTIN_TYPES`登记 bare
   `InternalError -> std.error.InternalError`，但仓库没有对应 source、api.yml export或
   Package schema record。
2. `runtime/eval/src/exceptions.rs::standard_error_type_identity`识别
   `std.service.ProviderUnavailableError`、`std.service.ProtocolError`及其它标准错误，但不识别
   `std.error.InternalError`或目标`std.service.InternalError`。
3. runtime/model/native/host中大量字符串 code `"InternalError"`属于 generic
   `RuntimeErrorPayload`诊断，不是一个用户可构造、可 public-schema encode、可 exact catch的 Skiff nominal
   type，不能做全局 rename。

`std/service.skiff`当前只声明`ProviderUnavailableError`与`ProtocolError`，
`std/api.yml`也只公开这两项。目标实现必须：

- 在`std/service.skiff`新增真实名义 record
  `InternalError { message: string, traceId: string, errorId: string }`；
- 在`std/api.yml`显式 public，使其获得`PublicNameable + SchemaClosed` Package schema identity；
- 把它加入唯一 platform/catch identity registry与固定 envelope codec；
- 删除不存在的`std.error.InternalError` compiler builtin，不创建旧路径 alias或兼容 root；
- 从所有标准错误删除`implements ErrorPayload`，不改变它们原有 nominal identity。

当前`standard_error_type_identity`的完整 catchable builtin集合是：

```text
CancelError
TimeoutError
config.DecodeError
std.bytes.DecodeError
std.number.DecodeError
std.json.DecodeError
std.db.ConflictError
std.db.DecodeError
std.file.FileError
std.resource.ResourceError
std.time.DecodeError
std.service.ProviderUnavailableError
std.service.ProtocolError
std.http.HttpError
```

它相对权威`std-surface.md`还多出`std.resource.ResourceError`、缺少
`std.service.InternalError`。`std.resource.ResourceError`有真实 source与 api.yml public export，因此仍是
普通可抛/可捕获的 std Package nominal；但它不应仅因旧 hard-coded表而进入 fixed`PlatformError` allowlist。
跨 service时应按普通 owner-Package public error规则处理。该差异由权威列表直接裁决，不构成新的用户设计
选择。

### 3. language checkpoint必须提供的事实

一个共享 checkpoint必须同时产出，不能让 compiler/runtime各自推导：

- declaration kind：nominal record、nominal representation、named union、transparent alias、interface；
- 完整实例化后的 nominal type identity；
- named union enclosing context；
- concrete nominal、anonymous discriminator synthetic与literal branch identity；
- throw instruction source span；
- call instruction source span；
- static `CatchLeaves`验证与 throw “所有可能运行时值都有 identity”验证；
- rethrow operand是`Exception<E>`且`CatchLeaves(E)`非空的验证。
- File IR `catch_type`改为 required；不存在 untyped catch-all或 empty-leaf wildcard。

`payload_type`仍应保留在`StmtIr::Throw`、`ExprIr::Throw`与 test-effect throw IR中；它不再表示 declared
public throw set，而是执行时选择 actual catch identity所需的静态类型事实。rethrow不增加 throw site。

## 三、Artifact 与 compiler owner 审计

### 1. 可以严格删除的 closed throw-set surface

以下字段及只为它们服务的逻辑应一次删除：

- `artifact-model/src/package_artifact.rs::PackageCallableSignature::throw_types`
- `artifact-model/src/boundary/operation.rs::BoundaryErrorContract`
- `artifact-model/src/boundary/operation.rs::BoundaryOperationContract::errors`
- callable signature的 throw type normalization、dependency identity rebinding、public surface validation；
- package boundary projection的 0/1/N throw-type→error-contract逻辑；
- contract operation error type normalization、existential validation与 schema root collection；
- deployment error value-plan eligibility分支；
- loader、compiler contract与 test-runner从 operation error收集 schema closure的分支；
- source inline-effect从 contract errors重建`PackageCallableSignature.throw_types`的逻辑。

同一 strict File IR cut还应删除`ExprIr::Catch.catch_type`的 optional/`None`形状；这不是 public error set
字段，但它是开放 opaque forwarding前必须关闭的 legacy wildcard。

具体 owner：

- `compiler/projection/src/package_artifact/callables/normalization.rs::normalize_public_signature`
- `compiler/driver/source_compile/canonical_dependencies.rs::bind_callable_signature_identity`
- `artifact-identity/src/package_artifact/validation.rs::validate_callable_surfaces`
- `compiler/projection/src/package_artifact/boundary/types.rs::project_operation_contract`
- `artifact-identity/src/contract/normalization.rs::normalize_contract_operation_contract`
- `artifact-identity/src/contract.rs::{validate_operation_existentials,collect_operation_refs}`
- `compiler/contract/src/projection.rs::collect_operation_refs`
- `deployment/src/projection/eligibility.rs::validate_contract_features`
- `runtime/loader/src/runtime_assembly.rs`的 operation error schema root
- `test-runner/src/package_schema_contract.rs::collect_operation_refs`

### 2. 必须保留但重新解释清楚的 throw facts

以下不是 public throw set，不能顺带删除：

- `artifact-model/src/executable.rs::{StmtIr::Throw,ExprIr::Throw,TestEffectOutcomeIr::Throw}`
  的`payload_type`；
- `artifact-model/src/boundary/projection.rs::CallableProvenanceSummary::Analyzed.throw_origins`；
- `artifact-model/src/effects.rs::CallableMayEffects::throws_caller_alias`；
- `artifact-model/src/boundary/operation.rs::BoundaryEffectGuarantee::detached_error`；
- `compiler/source/src/callable_effects/provenance.rs::{CallableState::throw_origins,record_wire_detached_throw}`；
- expression/statement/call transfer中的 throw provenance；
- `compiler/projection/src/package_artifact/boundary/eligibility.rs`对 caller-alias throw的 boundary安全检查。

它们回答的是“异常payload是否引用 caller heap、boundary能否安全 detach”，不是“operation会抛哪些公开类型”。

`compiler/source/src/callable_effects/transfer/call.rs::detached_contract_callee`当前要求
`contract.errors == None`才接受 detached contract，并且返回 state不记录 throw origin。开放通道后：

- 删除对`errors`的检查；
- service call无条件具有可能的 detached error completion；
- 应以`Fresh` throw origin或等价结构表达它，不引入类型集合；
- 继续要求`detached_error`与其它 boundary effect guarantee；
- 该窄改动必须直接依赖 F278完成后的 checkpoint，不能重写或重新评审 F278 的 same-heap owner模型。

### 3. inline effects / doubles

当前链：

- `compiler/source/src/expression_type_model.rs::check_test_effect_throw`要求值恰好匹配一个 declared
  `throw_type`；
- `compiler/lowering/src/function_lowering.rs`读取`test_effect_throw_payload_type`；
- `runtime/eval/src/test_effect_registry.rs::RuntimeTestEffectRegistry::dispatch`构造
  `UserException`；
- `runtime/eval/src/eval_context.rs`在正常 service dispatch之前直接返回 registry结果。

注册阶段同样有 eager-wire遮挡：`eval_context.rs`处理`LinkedTestEffectOutcomeIr::Throw`时立即用
`runtime_to_wire_required_plan`把 payload存进`RegisteredTestEffectOutcome::Throw`，dispatch时再 decode/encode
一次。新 registry storage不能借“不能跨 setup heap保留 handle”重新施加 SchemaClosed：
真正 local target需要 deferred/request-local carrier；service/host-boundary target可以消费同一个 canonical
export plan并在实际 dispatch时创建 correlation/exception。具体 storage布局属于实现细节，但不能建立第二套
错误分类。

目标行为：

- `throw:`接受任意符合语言 throw规则的 nominal值，不读取 Package/Service signature throw set；
- 真正的 package-direct double仍按本地/package call执行；模拟 service或 host boundary的 double必须经过与
  真实 boundary相同的 public-preserve/InternalError export/import流程，不能只因 target以 Package callable
  表示就绕过 boundary；
- test runner不再因 operation error收集 Package schema；
- service double不能成为绕过 public/private、SchemaClosed、encode failure、new caller stack或脱敏的后门。

### 4. strict schema 与 identity影响

必须在一个 checkpoint明确 bump受影响的版本/identity domain：

- File IR：`artifact-model/src/schema.rs::{FILE_IR_SCHEMA_VERSION,FILE_IR_FORMAT_VERSION}`与
  `artifact-identity/src/constants.rs::FILE_IR_IDENTITY_PREFIX`，因为 declaration kind、branch identity和
  instruction source span改变 canonical wire/preimage。
- PackageArtifact：`PACKAGE_ARTIFACT_SCHEMA_VERSION`以及
  `PACKAGE_ARTIFACT_BUILD_IDENTITY_*`、`PACKAGE_ARTIFACT_LOCAL_ABI_IDENTITY_*` marker/prefix，
  因为 public callable signature删除 field，boundary projection也删除 field。
- code-free contract authoring与 canonical contract：
  `SERVICE_CONTRACT_DEFINITION_SCHEMA_VERSION`、`SERVICE_CONTRACT_SCHEMA_VERSION`。
- ServiceProtocolIdentity：`SERVICE_PROTOCOL_IDENTITY_SCHEMA_MARKER/PREFIX`，因为当前 preimage序列化完整
  operation，删除恒为`None`的`errors`仍是一次真实 shape change。切换后，未来新增错误类型不再改变该
  identity。
- runtime frame：`runtime/transport/src/protocol.rs::RUNTIME_FRAME_SCHEMA_VERSION`，因为 service
  `response.error`不再是 generic`code/message/status/details`。

明确不应因本任务改语义的 domain：

- `ContractOperationId`只由 service id + stable operation key派生，不变；
- Publication ABI / operation ABI本来没有 declared throw set，不变；
- Package schema type identity算法不变；只新增`std.service.InternalError`自己的 record/index/build；
- runtime assembly/service build等引用上游 identity的产物会自然变化，但若其自身 preimage schema未变，
  不应仅为“引用值变了”额外复制版本规则。

所有 DTO继续`deny_unknown_fields`。新 reader拒绝旧`throwTypes/errors`，旧 reader自然拒绝新 schema；
不加 serde default、不读取旧版本、不双写、不做 migration adapter。

## 四、Runtime、schema、wire 与 observability owner 审计

### 1. 已有 schema/materialization可复用的部分

Package public error不需要新建第二套 schema语言：

- `compiler/projection/src/package_artifact/schema.rs::project_package_schema`已经只为 api.yml显式 public、
  非透明 alias、`SchemaClosed`类型生成
  `packageId/stableSchemaKey/PackageSchemaTypeId/canonicalDescriptor`。
- `runtime/loader/src/runtime_assembly.rs::{load_packages,load_package_schema_closure}`已按 code slot加载并验证
  Package自己及依赖引用的 exact schema record closure；但
  `RuntimeAssemblyContentResolver`还没有`resolve_package_schema_index`，这是下节列出的缺口。
- `runtime/linked-program/src/shared_image.rs::SharedPackageCode`保留 artifact、files与 schema records。
- `runtime/linker/src/assembly_execution/indexes.rs::build_execution_type_index`已把每个 code slot的
  `implementation_links.types` public path映射到`TypeAddr`。
- `runtime/boundary/src/service_value_plan/compile.rs::ServiceValuePlanCompiler`已严格核对
  owner/key/type id，并能编译 record、representation、discriminated union及`union_branch` plan。
- `CanonicalServiceBoundaryPlan`的 fresh heap、parameter/return directional materialization与
  capability hooks可继续使用。

因此 owner Package可以是 throwing service package，也可以是它的任意 dependency：actual `TypeAddr.unit`
已经指向真正 package code slot，不应把 identity改写成 throwing service owner。

### 2. 缺失的 assembly-owned双向 index

compiler publication已经通过
`compiler/driver/authoring/package_publication.rs`把完整`PackageSchemaIndex`写入 canonical store，但
`PackageArtifact.package_schema_index`只保存`PackageSchemaIndexRef`，当前 runtime loader resolver没有读取
该 index。`RuntimeTypeContext`则只有：

- `TypeAddr -> TypeDeclIr`
- `(packageSlot, publicPath) -> TypeAddr`

它没有：

- `TypeAddr -> (packageId, stableSchemaKey, PackageSchemaTypeId, codec plan)`
- `PackageSchemaTypeId -> caller-linked execution address set + canonical concrete/union branch identity`

runtime loader必须先按`PackageSchemaIndexRef`解析并验证 exact index，再由 linker以同一 package code
slot内的：

```text
Resolved PackageSchemaIndex.types[stableSchemaKey]
    -> { packageSchemaTypeId, publicPath, PublicNameable }
  + PackageArtifact.implementation_links.types[publicPath]
  + SharedPackageCode.schema_records[PackageSchemaTypeId]
```

构建唯一`ServiceErrorTypeIndex`。同一 exact owner/key/id record可按 content identity去重；同一
PackageSchemaTypeId的冲突 record、同一 execution address的多重 public identity、owner/key/id不一致、
public path没有 execution type、record缺失或 descriptor不一致都在 load/link admission失败关闭。一个
assembly若因不同 build含多个等价 execution address，应把它们规范到同一 Package schema catch identity，
再按 caller已链接的地址 materialize，不能把“地址多于一个”误报成 schema collision。更不能等到 throw时按
type name、display string、record shape或 throwing service contract猜测。

operation contract scoped schema records在删除`errors`后不再是 error lookup owner；错误类型自己的 Package
artifact/index才是 owner。

### 3. 最小共享 error abstraction

固定 DTO应只有一个 Rust canonical owner，建议位于低层`runtime/model`，形状严格等于权威架构：

```text
ServiceErrorEnvelope
  = PublicTypedError {
      packageId,
      stableSchemaKey,
      packageSchemaTypeId,
      encodedPayload,
      traceId,
      errorId
    }
  | InternalError {
      payload: std.service.InternalError
    }
  | PlatformError {
      builtinErrorIdentity,
      encodedPayload,
      traceId,
      errorId
    }
```

eval/boundary层只应有一个 orchestrator，例如`CanonicalServiceErrorChannel`，邻接
`CanonicalServiceBoundaryPlan`并提供两种方向：

- `export_provider_failure`
  - unwrap本地 diagnostic，记录本 service完整本地栈到受限 telemetry；
  - 若已经携带 inbound fixed envelope且未被 catch/replaced，原样转发；
  - 对新用户错误用`ServiceErrorTypeIndex`查 actual owner；
  - public + PublicNameable + SchemaClosed + encode成功时生成`PublicTypedError`；
  - private/non-nameable/nonclosed/实际 encode failure第一次出界时生成一次
    `std.service.InternalError`；
  - catchable platform error生成`PlatformError`；
  - artifact/schema不变量损坏与 malformed inbound envelope失败为 Protocol/InvalidArtifact，不能伪装成
    用户 InternalError。
- `import_caller_failure`
  - caller链接 exact type时按 owner record decode并恢复名义/branch/context identity；
  - caller未链接 public type时保留 opaque envelope，任何 local catch都不匹配，但 outward export可原样
    forward；
  - 为当前 service call site创建新的 request-local`Exception<E>`、新的 caller stack与一帧只含安全
    service/operation/errorId信息的 remote-boundary frame；
  - 不导入 callee source path、function name、diagnostic wrapper或`Exception`对象。

同一 orchestrator必须被 ordinary、async/stream/cancel、ingress、service test effect与未来
RemoteBoundary消费。codec可下沉复用`runtime/boundary`，但分类、InternalError生成、opaque forwarding、
correlation与本地 stack不能在 eval、host、router各复制一份。

`UserException`需要能区分：

- 已 materialize的本地 actual nominal/branch identity与 request-local runtime value/handle；本地
  catch/rethrow不得先 wire encode；
- 未链接但可透明转发的 opaque `ServiceErrorEnvelope`；
- 当前 request-local source/stack。

opaque public error不是可结构化检查的`RuntimeValue`，也不是“unknown record”；catch只在 exact linked identity
恢复后匹配。

### 4. `InternalError`生成一次与每跳新栈

request-scoped error context应持有当前 trace、error id generator、activation/service/operation与 telemetry
sink：

- 初始 throw/catchable platform cause取得或创建唯一`errorId`；
- private/nonclosed/encode-failed error第一次出界时以同一 cause生成固定
  `InternalError { fixed message, traceId, errorId }`；
- B收到后若不 catch，B→C直接发送同一 envelope、同一 payload、同一 traceId/errorId；
- 每次 import只创建本 service自己的 exception/stack；
- 同一 request中的 rethrow复用现有本地 envelope与 throw site；
- 如果用户显式抛出已 materialize的`std.service.InternalError`，也不得再套第二层。

“公开但 assembly缺 record/owner index”的情况是损坏 artifact/admission，不是普通 private error；
“record/plan合法但实际值无法编码”才按权威规则转换成 InternalError。

### 5. source、stack、trace 与 telemetry现状

source/stack为空的直接原因：

- `UserException::from_typed_payload`硬编码`source = null`、`stack = []`；
- `artifact-model/src/executable.rs`的 throw/call IR没有 instruction span；
- `runtime/eval/src/program_execution.rs::attach_program_source_context`要求`source_id`，但
  `runtime/eval/src/{eval_context.rs,program_invocation.rs}`所有 production call site都传`None`；
- `runtime/eval/src/source_context.rs::program_source_context_frame`已有从 source map构 frame的能力，却没有
  instruction提供 id。

diagnostic泄露风险：

- `runtime/eval/src/error.rs`与`runtime/host/src/error.rs`的
  `add_source_frame/add_diagnostic_frame`会把`sourceId/sourceFrame/sourceFrames/frames`合入
  `RuntimeErrorPayload.details`；
- generic`response.error`会把 details发到 router；
- Router最终隐藏 HTTP 5xx detail并不等于 runtime wire没有泄露。

trace/error telemetry缺口：

- Router request frame已有 trace；
- `runtime/host/src/host/request_trace.rs::RequestTraceFields`只在
  `host/control_plane.rs`的 route-error event应用；
- `runtime/host/src/host/request_entry/assembly.rs::assembly_request_telemetry_context`填 service/build/
  activation/runtime/request/target，却不复制 traceId/spanId/parentSpanId；
- `runtime/transport/src/protocol.rs::TelemetryEvent`与
  `router/src/protocol/envelope.ts::TelemetryEvent`有 trace fields，但没有权威 Event Shape要求的
  top-level errorId；
- `runtime/host/src/host/request_supervisor.rs::complete_error`只接收与外部 wire共用的
  `ResponseError`，再经`runtime/request/src/runner.rs::response_error_to_telemetry_map`写 telemetry：
  若保留 diagnostic details就会让 wire有泄露风险，若先脱敏又会丢完整本地 stack；当前没有独立 restricted
  diagnostic owner。

目标必须把“外部 service response”与“受限本地 diagnostic event”分开：

- response只携带 fixed envelope及安全 remote metadata；
- restricted telemetry event携带当前 service完整 stack/source引用、traceId、errorId；
- telemetry Rust DTO、router protocol validator与存储/转发镜像增加同一 errorId字段；
- telemetry serializer消费 runtime已分类的 canonical cause，不能再次按 message/code猜 error类型。

### 6. wire/router边界

service `response.error`必须从 generic`RuntimeErrorFramePayload`切换到严格 fixed envelope；具体把
`encodedPayload`放在 typed header还是 binary payload是 checkpoint内的实现布局，不改变公共语义，但必须
满足：

- 一帧只能有一个明确 envelope与payload owner；
- Rust encoder/decoder是 canonical owner；
- TypeScript只做严格 schema parity、opaque forwarding与外部 HTTP/WebSocket映射；
- unknown variant、extra/missing field、payload presence不一致、owner/key/type id不一致全部拒绝；
- router不维护 public/private、SchemaClosed、platform allowlist或 InternalError转换规则；
- pre-ingress gateway decode/route failure仍可使用 gateway/control error surface，不应把所有
  control-plane`RuntimeErrorPayload`盲目改成业务可 catch error；
- old runtime frame v1不兼容读取。

## 五、测试与公共生态 consumer

### 1. Skiff repo内必须迁移的 consumer类别

- std/prelude source与 source-layout checker；
- compiler semantic/lowering/package schema fixtures；
- artifact model/identity/projection/driver/deployment中构造
  `PackageCallableSignature { throw_types }`或
  `BoundaryOperationContract { errors }`的 fixtures；
- runtime loader/linker/eval/host/package-test中的同类 fixtures；
- inline effect E2E与 test-runner canonical native stubs；
- runtime/host与 router对`UnhandledServiceError`的断言；
- runtime transport/router protocol的`response.error`与 telemetry parity tests；
- live DB fixture中的`implements ErrorPayload`。

co-located fixture由其 production owner同一节点迁移；不要保留“只让旧 fixture继续读”的 adapter。
`UnhandledServiceError`的 production owner是`runtime/eval/src/error.rs::user_exception_payload`，直接断言
集中在`runtime/host/src/error/tests.rs`与`router/tests/test-dispatch.test.ts`；不能只改测试字符串而保留旧
flattening。

### 2. 公共生态只读结果

`/Users/geek/workspace/skiff-packages`没有`ErrorPayload`、`throw_types`、
`BoundaryErrorContract`或`UnhandledServiceError` consumer；现有 catch selector也只包含标准 nominal或其
anonymous union，不需要迁移任务。

`/Users/geek/workspace/internals`有十个 Skiff nominal声明仍写 marker：

- `packages/llm-api/decode.skiff::LlmDecodeError`
- `packages/agent/drain.skiff::{DrainCheckpointConflict,ProviderStreamFinishError}`
- `packages/agent/runner.skiff::RunnerTransactionConflict`
- `packages/agent/thread_runtime_support.skiff::{AgentInputRetryError,AgentControlRetryError,AgentToolRejectedError}`
- `packages/agent/tools.skiff::RuntimeBindingError`
- `agine/service/api/agine.skiff::ApiError`
- `agine/service/internal/host_tool_settlement_store.skiff::HostSettlementRetryError`

internals没有`throw_types`、`BoundaryErrorContract`或`UnhandledServiceError`consumer；这里需要迁移的是 source
marker与受新合法性检查暴露出的真实非法 leaf，不是 artifact/wire DTO。

对现有`catch<...>` selector按类型形状复核后，它们都是上述用户 nominal、标准 nominal或这些 nominal的
anonymous union；没有 primitive/interface/unknown/anonymous-record catch selector，也没有直接 throw
literal/primitive的命中。因此当前可预见 source改动就是十处 marker，最终仍须由新 compiler focused compile
证明，而不是依赖搜索数量放行。

其中`packages/llm-api/api.yml`显式公开`LlmDecodeError`，
`packages/agent/api.yml`显式公开`RuntimeBindingError`，适合作为第三方 owner Package真实路径；
`agine/service/api.yml`没有直接公开`ApiError`，不能把它假定为可保留的 public跨服务错误。

internals内大量 local throw/catch仍应保持名义行为，迁移只删除 marker并按新 compile diagnostics修正真正
非法 leaf。TypeScript名称`ChatErrorPayload`、`ToolInvocationErrorPayload`是业务 DTO，不是
`ErrorPayload` marker，禁止批量 rename。

跨仓改动必须独立 commit；本审计不修改 internals。

## 六、canonical owner处置

### 1. 删除

- compiler-known`std.error.ErrorPayload` interface与 bare prelude builtin；
-所有`implements ErrorPayload`及 native test stub；
-不存在的`std.error.InternalError` builtin；
- `PackageCallableSignature.throw_types`；
- `BoundaryErrorContract`与`BoundaryOperationContract.errors`；
- error type normalization/validation/projection/contract schema roots；
- declared-throw驱动的 inline effect规则；
- File IR optional`catch_type: None`与 runtime empty-leaf catch-all；
- `UnhandledServiceError` user-exception flattening路径；
- legacy service error→ProviderUnavailable语义 mapper，或把 legacy path整体退役。

### 2. 复用

- parser discriminator合法性检查；
- Package public schema index/type record与 SchemaClosed验证；
- package code slot、implementation type export与 loaded schema records；
- runtime boundary codec/value plan；
- fresh provider heap与 directional materialization；
- exact `TypeIdentity`比较的基本原则；
- rethrow复用同一 semantic exception/cause的控制流原则，不复用当前 JSON round-trip实现；
- diagnostic wrappers作为**本地受限 telemetry输入**；
- throw provenance、caller-alias effect与`detached_error`保证；
- Router ingress trace。

### 3. 新增唯一 owner

- artifact-model declaration/branch/source-span schema；
- compiler source `CatchLeaves`与 throw/rethrow validator；
- real public`std.service.InternalError`；
- runtime-model `ServiceErrorEnvelope`与统一 platform identity registry；
- linker/assembly `ServiceErrorTypeIndex`；
- eval/boundary `CanonicalServiceErrorChannel`；
- request-local error correlation/stack context；
- service response error wire与 Rust/TS严格 parity；
- restricted telemetry error event的 errorId与本地 stack owner。

### 4. 明确禁止复制

- 不在 ServiceContract、operation signature或 Package callable signature复制 error type set；
- 不在 throwing service artifact复制 dependency-owned schema record作为自己的 error；
- 不在 router/TypeScript复制 public/private、SchemaClosed或 platform分类器；
- 不在 ordinary/stream/ingress/test effect/remote各写一套 InternalError转换；
- 不按 display name、module string、record shape或 discriminator猜 nominal identity；
- 不把 callee `Exception`/diagnostic stack复制到 caller；
- 不为中间 service decode/re-encode opaque public envelope；
- 不把 generic runtime code `"InternalError"`当成`std.service.InternalError`做全局替换；
- 不为旧 artifact/wire增加 default、fallback、legacy adapter或 dual write。

## 七、三波实现 DAG

所有节点从同一 integration分支顺序整合。Wave 2节点只在 Wave 1 schema checkpoint冻结后并行；其中会触及
F278相邻 compiler文件的节点还必须等 F278先整合。Wave 3只在所有 Wave 2节点通过各自 focused
acceptance后开始。

### Wave 1：共享 strict model/schema checkpoint

#### `W1-S Shared error identity and wire model`

- 直接依赖：`512135dd`、F278已声明的写入面清单；不依赖 F278实现结果。
- production写入范围（唯一 owner）：
  - `artifact-model/src/{types.rs,executable.rs,package_artifact.rs,boundary/operation.rs,schema.rs,lib.rs}`
  - `runtime/model/src/{error.rs,value.rs,type_plan.rs,lib.rs}`及一个新的 service-error DTO module
  - `artifact-identity/src/constants.rs`
- test写入范围：
  - 仅上述 crates的 strict serde、golden、identity mutation tests。
- 交付：
  - declaration kind/branch context与 throw/call source span canonical shape；
  - required typed catch IR，删除 optional catch-all；
  - 删除`throw_types/errors`的严格 DTO；
  - fixed`ServiceErrorEnvelope`与 runtime catch identity数据模型；
  -受影响 schema/identity marker/prefix一次 bump；
  - 旧 field、source-owned instruction缺失新 required site fact、unknown envelope variant全部失败关闭；
    compiler/runtime生成的 synthetic call site必须使用显式 synthetic kind，而不是伪造 source path。
- 风险：最高。任何后续节点不得再改变这些 public/internal shared DTO；发现问题退回 checkpoint owner。
- acceptance组：`A1 strict-model`，只跑 artifact-model/runtime-model/artifact-identity focused tests。

该 checkpoint可以是短暂的 integration break point，但在 fan-out前必须冻结完整 shape；不能让各节点各自加临时
兼容 field来保持旧编译。

### Wave 2：非重叠 production fan-out

#### `W2-L Language, std and lowering`

- 直接依赖：`W1-S`。
- production写入范围：
  - `syntax/**`
  - `compiler/core/**`
  - `compiler/source/**`，**排除**`compiler/source/src/callable_effects/**`
  - `compiler/lowering/**`
  - `prelude/**`、`std/**`、`std/api.yml`
  - `scripts/check-skiff-source-layout.mjs`、`vscode/**`
- test写入范围：上述目录co-located compiler/static/lowering/std/tooling fixtures。
- 交付：static CatchLeaves/throw/rethrow规则、nominal representation与named-union branch identity lowering、
  throw/call spans、real public InternalError、删除 marker与tooling特殊项、任意语言合法 nominal inline-effect
  payload选择。
- 风险：高，尤其 generic instantiation、same-shape union与source map determinism。
- acceptance组：`A2-language`。

#### `W2-A Artifact, contract and dependency consumers`

- 直接依赖：`W1-S`与已整合 F278；原因仅是
  `compiler/projection/src/package_artifact/callables/normalization.rs`同时承载 signature与 semantic-facts
  normalization，本节点消费 F278结果但不重新设计它。
- production写入范围：
  - `artifact-identity/**`，排除`artifact-identity/src/constants.rs`
  - `compiler/compiled/**`
  - `compiler/projection-input/**`
  - `compiler/projection/**`中 closed throw-set/contract consumer；保留 F278已落地的 semantic-facts形状，
    不改 same-heap语义
  - `compiler/contract/**`
  - `compiler/driver/source_compile/canonical_dependencies.rs`
  - `deployment/**`
  - `test-runner/src/package_schema_contract.rs`
- test写入范围：上述目录co-located strict artifact/contract/deployment fixtures。
- 交付：删除 closed throw-set所有 producer/normalizer/validator/root，重新生成 Local ABI与
  ServiceProtocolIdentity golden，证明 Operation ABI/Publication ABI不含 error set。
- 风险：高；漏一个 schema-root consumer会让 error owner错误地继续依赖 operation contract。
- acceptance组：`A3-artifact-contract`。

#### `W2-E Open-channel effect consumer`

- 直接依赖：`W1-S`与**已完成/整合的 F278**。
- production写入范围：
  - 仅`compiler/source/src/callable_effects/transfer/call.rs`中
    `detached_contract_callee`及必要的最小相邻 helper。
- test写入范围：
  - 仅`compiler/source/src/callable_effects/tests.rs`新增 open-channel detached throw cases。
- 交付：移除`errors == None`条件、保留全部 detached/same-heap保证、把任意 service call error建模成
  Fresh detached throw origin。
- 风险：中高；不得改变 F278 owner/path/token模型或 boundary availability阈值。
- acceptance组：`A4-effects`。

#### `W2-R Runtime identity, codec and in-process channel`

- 直接依赖：`W1-S`；集成验收依赖`W2-L`、`W2-A`。
- production写入范围：
  - `runtime/linked-program/**`
  - `runtime/loader/**`
  - `runtime/linker/**`
  - `runtime/boundary/**`
  - `runtime/eval/**`
  - `runtime/capability-context/**`中 service response carrier
- test写入范围：上述 runtime crates的co-located tests；不触碰 host/transport/router。
- 交付：
  - linked declaration/branch identity；
  - assembly-owned`ServiceErrorTypeIndex`；
  - canonical export/import/opaque-forward orchestrator；
  - 不经 wire的 request-local exception payload、initial throw与per-hop local stack；
  - ordinary/async/stream/cancel/ingress以及 service/host-boundary test-effect一致语义；
  - legacy outbound不再成为第二分类 owner。
- 风险：最高；heap隔离、dependency owner、opaque forwarding、encode failure和diagnostic隔离都在此节点。
- acceptance组：`A5-runtime-channel`。

#### `W2-W Transport, host, router and telemetry`

- 直接依赖：`W1-S`；集成验收依赖`W2-R`。
- production写入范围：
  - `runtime/request-contract/**`
  - `runtime/request/**`
  - `runtime/transport/**`
  - `runtime/host/**`
  - `router/**`
  - `telemetry/**`中 protocol/validation/storage consumer
- test写入范围：上述 crates/packages的co-located protocol、host、router、telemetry tests。
- 交付：strict response.error v2、Rust canonical encode/decode、TS parity、external redaction、trace propagation、
  top-level errorId、本地 restricted diagnostic event；删除`UnhandledServiceError`断言。
- 风险：高；必须区分 service response与 pre-ingress/control error，不能全局替换所有 generic error DTO。
- acceptance组：`A6-wire-observability`。

Wave 2的 F280节点写入范围互不重叠；`W2-A`与`W2-E`都在 F278整合后启动，因而不会与仍在运行的 F278并发
编辑。任何未列出的 shared file只能由 integration owner显式重新分配，不能由两个节点同时修改。

### Wave 3：真实路径、生态迁移与唯一 gate

#### `W3-P Cross-layer real-path probes`

- 直接依赖：全部 Wave 2节点。
- production写入范围：无。
- test写入范围：新建一个专用 open-service-error-channel integration fixture/test目录；不回写 Wave 2
  co-located unit fixture。
- 交付：下节正负矩阵的 hermetic compiler→artifact→loader/linker→runtime→wire路径。
- 风险：高；必须使用真实 PackageArtifact/ServiceContract/assembly，不得手造绕过 projection的 typed
  contract。
- acceptance组：`A7-cross-layer`。

#### `W3-I Internals marker migration`

- 直接依赖：全部 Wave 2节点与`W3-P`语言/边界主路径通过。
- production写入范围：仅`/Users/geek/workspace/internals`上列十个`.skiff`声明；不修改同名 TypeScript
  domain DTO。
- test写入范围：internals中受影响 package/service的co-located tests。
- 交付：独立仓库 commit，删除 marker，确认 public `LlmDecodeError`/`RuntimeBindingError`仍获 schema
  record，private/nonpublic errors不被意外公开。
- 风险：中；跨仓提交与 push必须分别授权。
- acceptance组：`A8-ecosystem`。

#### `W3-G Integration and expensive-gate owner`

- 直接依赖：`W3-P`、`W3-I`以及全部 focused acceptance。
- production/test写入范围：无；只负责整合、最终状态检查与验收记录。
- 唯一昂贵 gate：
  - 在最终 Skiff integration state只运行一次`pnpm verify`；
  - 并行实现节点不得各自运行完整 gate；
  - 若后续实现任务明确授权 stable/跨仓验收，同一 owner按 workspace约定 build/restart stable runtime后，
    在`/Users/geek/workspace/internals/agine`只运行一次`npm run e2e:chat-smoke`；
  - live/stable/chat smoke不属于本审计或默认并行实现授权，未明确授权时不运行。
- 风险：中；不得用重复 full gate掩盖 focused probe缺失。
- acceptance组：`A9-final`。

## 八、最早便宜探针与最终正负矩阵

### 1. 每个高风险点最早运行的便宜探针

1. `W1-S`一落地：
   - artifact serde负例拒绝旧`throwTypes/errors`、optional/missing catch type、unknown field、缺失
     declaration kind/source-owned instruction span；
   - fixed envelope三 variant round-trip与 unknown/missing field拒绝；
   - identity mutation test证明 File IR、PackageArtifact Local ABI/build、ServiceProtocol identity发生一次预期
     变化，Operation ABI与无关 PackageSchemaTypeId不变。
2. `W2-L`首个可编译状态：
   - source micro-fixture覆盖 record、representation、transparent alias、合法 anonymous nominal union、
     named union named branch、discriminator branch、literal branch；
   - compile-fail覆盖 primitive、anonymous record、container、interface、unknown、function、unconstrained
     generic、mixed/non-nominal union、nullable catch/throw与非`Exception<E>` rethrow；
   - File IR golden断言 branch/context identity与 throw/call source span稳定。
3. `W2-A`：
   - producer artifact JSON不含`throwTypes/errors`；
   - 手工加入旧 field立即 strict decode失败；
   - mutation matrix证明 changing provider body error不改变 ServiceProtocolIdentity；
   - contract schema closure只含 param/return/stream/callback roots。
4. `W2-E`：
   - detached service contract state的`throw_origins == [Fresh]`；
   - caller alias不因 open channel凭空出现；
   - F278 same-heap正负 fixture结果不变。
5. `W2-R`：
   - linker index单测：own package、dependency owner、跨 build exact record去重、冲突/错配 owner/key/id、
     missing export；
   - pure codec单测：public encode/decode、private→InternalError、nonclosed→InternalError、encode failure、
     already-fixed envelope原样 forward、malformed envelope→Protocol；
   - exception单测：含 nonclosed/capability字段的本地 error可 throw/catch/rethrow且不调用 boundary encoder、
     initial source/stack、same-request rethrow、per-hop new stack、opaque catch miss。
6. `W2-W`：
   - Rust response.error encode/decode与 TS validator用同一 golden corpus；
   - payload presence、unknown variant、extra field、schema v1全部负例；
   - telemetry event含 traceId/errorId，本地 stack只在 restricted event；
   - runtime wire与 router HTTP body不含 source path/function/private payload。

这些 probe用 focused crate/package selector或单 test filter运行；不提前运行`pnpm verify`，不启动 instance。

### 2. 最小真实路径矩阵

| ID | 真实拓扑/输入 | 正向断言 | 配对负向断言 |
| --- | --- | --- | --- |
| L0 | 单 Package私有、含 non-`SchemaClosed`字段的 nominal error本地 throw/catch/rethrow | 全程只用 request-local value/exception carrier，不要求 encode | generic`runtime_to_wire`不得在 initial throw或 local rethrow被调用 |
| L1 | 单 Package私有名义 record本地 throw/catch/rethrow | exact catch成功；initial stack有 throw site；rethrow同 envelope/id/site | 同 shape不同 nominal type不匹配 |
| L2 | 私有 representation本地 throw/catch | catch representation identity成功，payload仍按 RHS使用 | transparent alias只展开 RHS；primitive RHS不能直接成为 leaf |
| L3 | named union含 concrete、anonymous discriminator、literal branches | 每个 branch有稳定 identity并保留 enclosing union context；catch union成功 | 同 shape/tag的另一个 named union不匹配 |
| L4 | 非法 catch/throw/rethrow集合 | compile-time失败关闭 | 不允许推迟成 runtime Decode |
| B1 | Provider与caller都链接同一 Package public error | `PublicTypedError`保留 exact owner/key/type id，caller恢复 nominal value并 catch | owner/key/type id任一篡改都 Protocol，不按 shape catch |
| B2 | Provider抛自己 dependency Package公开的 error，caller也链接 dependency | envelope owner是 dependency，不是 provider；caller exact catch | throwing service不能复制/改写 owner |
| B3 | C→B→A；A抛 public error，B未链接 owner，C链接 owner | B opaque、不 catch、不 decode，转发同 bytes/traceId/errorId；C恢复并 catch | B按字段/名字猜类型必须失败 |
| B4 | private error首次出界 | 生成一次固定`std.service.InternalError`；无原 type/字段/display泄露 | 后续未处理 hop不能再包一层或换 errorId |
| B5 | public但非`SchemaClosed`类型出界 | 与 B4相同的 InternalError语义 | 不能因为 api.yml有名字就发送半闭包 schema |
| B6 | public、plan合法但实际 payload encode失败 | 与 B4相同，并在 callee restricted telemetry保留本地诊断 | 不能把 encoder message/原值送上 wire |
| B7 | public schema record/index/export缺失或冲突 | load/link admission失败，或 malformed inbound为 ProtocolError | 不能降级成普通 InternalError掩盖损坏 artifact |
| B8 | `std.db.ConflictError`或`std.service.ProviderUnavailableError`跨 service | 固定`PlatformError`、exact builtin catch、同 correlation | router不能自己按 code重新分类 |
| B8a | `std.resource.ResourceError`跨 service | 作为 owner为`skiff.run/std`的普通`PublicTypedError`编码，caller exact nominal catch | 不能因旧 hard-coded map把它误放进`PlatformError` |
| B9 | 已生成 InternalError经未处理 middle service | 同 payload/traceId/errorId透明传播 | 不重新 sanitize或生成新 cause |
| S1 | A初始 throw，B call A，C call B | A/B/C telemetry各有自己的完整本地栈；B/C local exception各有本 hop call-site stack和安全 remote frame | 任一 service response wire都不含 callee source id/path/function |
| S2 | same-request local rethrow与cross-service import对照 | local rethrow同 stack/site；remote import新 local stack | remote不能复用 callee `Exception` |
| T1 | 真正 package-direct inline effect throw | 服从语言 nominal规则并保持普通 package call的本地异常语义 | 不要求 declared throw set |
| T2 | service或 host-boundary inline effect throw | 走 public/private/encode/opaque/per-hop stack同一 channel | registry不能直接在 caller heap返回 raw UserException，也不能因 Package-shaped target绕过 boundary |
| W1 | runtime response.error经 router到外部 HTTP/WebSocket | internal frame严格 fixed envelope；外部响应按 gateway policy脱敏；trace/error可关联 | v1 generic frame、extra details/sourceFrames被拒绝 |

`W3-P`至少实现 L0–L4、B1–B9（含 B8a）、S1–S2、T1–T2、W1；不能用手写
`BoundaryErrorContract::Typed`伪造成功路径。

## 九、用户设计缺口

没有新增设计决策。

以下只是在既定语义内由`W1-S`冻结的实现布局，不需要用户选择：

- declaration/branch identity在 Rust DTO中的具体 enum/field命名；
- `encodedPayload`在 response.error header与 binary payload之间的物理布局；
- error id使用的随机 ID实现；
- restricted telemetry内部 frame DTO的字段组织。

它们都必须满足父结果与权威架构的 fixed envelope、exact owner、opaque forwarding、逐跳新栈和脱敏约束，
不能借“实现选择”引入第二语义 owner或兼容路径。

## 十、自验收

- 已按 build/admission与 request执行真实顺序定位首次损失；
- 已覆盖 language/std、artifact/compiler、runtime/wire、stack/trace/telemetry、test/ecosystem；
- 已区分可删除、可复用、必须新增及禁止复制的 owner；
- 已给出不超过三个波次、带直接依赖与非重叠写入范围的 DAG；
- 已给出最早 focused probe、最终跨层正负矩阵与昂贵 gate唯一 owner；
- 已明确 F278依赖/隔离与跨仓 commit边界；
- 已明确没有新增设计决策；
- 本审计未修改或执行任何非授权实现/验收面。
