# P5-F319 Service error channel delta audit结果

状态：只读审计完成；不构成W2-R验收，不给A5-runtime-channel PASS/FAIL。

证据基线：`6b8d52ed92a7b4db16f2a38e91673f1d8dff35b8`。

实际审计HEAD：`4a96897b8e20e43a9f1db089e6382a8606e71c58`。

从证据基线到实际审计HEAD，`runtime/**`、`artifact-model/**`和`compiler/**`没有production
diff；HEAD只新增本任务文档。因此下文是F298、F299、F305、F316已经整合后的当前production事实。
F318 representation eval仍是独立未整合节点；它只遮挡representation值经过真实eval构造器的探针，
不改变service error channel owner、依赖方向或其余结论。

## 审计结论

1. F298的`ServiceErrorTypeIndex`、F299的`RuntimeValueCarrier`/request-local
   `RequestException`以及F305的有限platform identity registry都已经进入真实production路径；但是三者
   尚未被一个service boundary owner汇合。F298 index已经由loader/linker真实构建并挂到
   `AssemblyExecutionImage`，production request执行没有任何consumer读取
   `execution_image.service_error_types()`。
2. `ServiceErrorEnvelope`、`OpaqueServiceError`和`ExceptionStackFrame::RemoteBoundary`当前只有
   runtime-model及其单测；普通、async、stream、cancel、ingress、test effect和legacy outbound没有一个
   production入口创建、解码或转发fixed envelope。
3. 当前ordinary/async unary provider error通过
   `CanonicalServiceBoundaryPlan::materialize_provider_error`原样返回。若它是
   `UserException`，caller侧`promote_platform_error_at_call`会原样保留provider的
   `RequestException`；它可能引用即将销毁的provider heap，并携带provider/caller混合stack。若它不是
   `UserException`但有platform projection，caller才在自己的heap和call site创建一个新的local
   exception。前者不是合法的boundary import，后者也没有fixed envelope/correlation透明传播。
4. canonical server stream比unary更早丢失所有权：provider task把
   `RuntimeError`直接装入`StreamRuntimeError::Producer(Box<dyn WirePayload>)`，既不保留provider heap，
   也不先固定成service envelope。普通同heap stream producer已有
   `RequestHeapOwnedStreamError`，但那只是local stream heap owner，不能当service error channel。
5. service `ContractOperation` test effect在真正target resolution和
   `dispatch_service_call`之前直接把setup heap payload deep-clone进caller heap并返回
   `UserException`，完整绕过provider heap、public/private/nonclosed/encode分类、fixed envelope与逐跳
   stack。`PackageCallable` effect应继续保持package-local语义，不能一并wire化。
6. 最小canonical owner应是`runtime/eval`中与
   `assembly_execution::boundary_materialization`相邻的新
   `assembly_execution::service_error_channel`模块。它消费低层model、boundary codec及linked image，
   不应落在`runtime/boundary`、`runtime/linker`、`runtime/model`、lane文件、host或router。
7. 当前两个必须先补的核心API事实是：
   - `RequestExceptionCause`必须能同时保留原始fixed envelope和可选caller-local materialized
     carrier；现有`Local`/`OpaqueService`二选一无法让“linked且可catch”的inbound error在未捕获时原
     bytes转发；
   - `ServiceValuePlan`必须提供exact named-union branch-aware binary encode/decode。底层binary codec
     已在payload首字节写/读branch ordinal，但现有公开API编码时按shape试分支、解码后丢弃ordinal，
     same-shape branches无法安全恢复。
8. 实现应拆成一个serial core checkpoint、三个互不重叠的parallel consumer和一个test-only
   convergence。W2-R只产出/消费Rust canonical fixed-error carrier；request/transport/host/router/
   telemetry的response.error v2及外部脱敏仍属于W2-W。

## Production跳点

| 真实阶段/入口 | 当前表示与owner | 当前下游或首次语义损失 | 与canonical channel的关系 |
| --- | --- | --- | --- |
| `runtime/loader/src/runtime_assembly.rs::{load_packages,load_package_schema_index,load_package_schema_closure}` | exact `PackageSchemaIndexRef`、完整record closure；loader | owner/key/type id、record引用、closure/content identity在admission校验 | 已是真实输入，直接复用 |
| `runtime/linker/src/assembly_execution/service_error_index.rs::build_service_error_type_index` | declaration/branch execution key↔Package public identity/record/context；linker | record、representation、named-union每个branch建row；generic/applied/unresolved fail closed | 已是真实producer，不复制到eval |
| `runtime/linked-program/src/assembly_execution.rs::AssemblyExecutionImage::service_error_types` | immutable assembly-owned index | production除构建/fixture外无人读取 | canonical channel的唯一index输入 |
| `runtime/eval/src/eval_context.rs::eval_program_throw` | `RuntimeValueCarrier`→`RequestException::local`→`RuntimeError::UserException` | exact local identity、site、stack、trace/error id保留；不编码 | 已正确，是export的local cause入口 |
| `runtime/eval/src/eval_context.rs::eval_program_catch`与`program_execution.rs::eval_program_rethrow_slot` | heap中的同一request-local exception | catch exact identity；rethrow复用cause/source/stack/correlation | 已正确；service import必须重新生成caller-local exception，不能改坏local rethrow |
| `runtime/eval/src/eval_context.rs::promote_platform_error_at_call`、`runtime/eval/src/exceptions.rs::request_exception_for_catch` | `WirePayload::catch_projection`→finite `CatchIdentity`→caller-local carrier | local platform catch真实可用；遇到已有`UserException`则完全不做boundary import | platform registry可复用，fixed export/import缺失 |
| `runtime/eval/src/assembly_execution/mod.rs::{dispatch_service_call,dispatch_in_process_boundary}` | resolved target后按ordinary或async/stream/cancel选lane | internal call和ingress共用此dispatcher；error仍只是`RuntimeError` | caller import/ingress fixed-error handoff的唯一汇合点 |
| `runtime/eval/src/assembly_execution/ordinary.rs::execute_service_call` | fresh provider heap；provider `Result<RuntimeValue,RuntimeError>` | success被detached；error经`materialize_provider_error`原样返回，provider heap随后drop | ordinary export必须在heap drop前进入唯一channel |
| `runtime/eval/src/assembly_execution/async_stream_cancel.rs::execute_provider_unary` | owned provider context+fresh heap | provider error同样原样返回；caller/request cancellation select直接返回`Cancelled` | provider terminal走channel；caller cancellation是control negative path，不伪造成provider envelope |
| `runtime/eval/src/assembly_execution/async_stream_cancel.rs::run_provider_stream` | `StreamRuntimeError::Producer(Box<dyn WirePayload>)` | provider error未带heap或fixed bytes进入sink；producer heap随task drop | stream terminal必须在task/heap存活时export |
| `BoundaryStreamSink::fail`、`Interpreter::exec_program_stream_for_in`、`runtime/eval/src/error.rs::materialize_stream_runtime_error` | sink原样转发`StreamRuntimeError`；local producer使用`RequestHeapOwnedStreamError`；consumer从dynamic `WirePayload` downcast | local stream能跨task clone heap；canonical provider stream没有使用它，且dynamic payload仍可按code投影 | service stream应传typed fixed carrier；local stream owner继续保留 |
| `runtime/eval/src/assembly_execution/ingress.rs::dispatch_ingress_via_in_process_boundary`及`websocket_ingress.rs` | synthetic call site进入同一dispatcher | success适配外部响应；error以generic `RuntimeError`向外冒泡 | W2-R只应交出fixed Rust carrier；外部frame/HTTP映射由W2-W消费 |
| `runtime/eval/src/test_effect_registry.rs::materialize_local_test_throw`和`eval_context.rs` call interception | setup `RuntimeValueCarrier`+setup heap→deep clone到caller heap→local `UserException` | `ContractOperation`在target resolution/boundary前返回；`PackageCallable`也走同一local helper | service effect必须export→import；package effect保持local |
| `runtime/capability-context/src/{response,outbound_response}.rs` | generic `ResponseError {code,message,status,details}`及`OutboundResponse::Error` | 没有fixed envelope或opaque bytes slot | 只能作为W2-W前的legacy seam，不能成为分类owner |
| `runtime/eval/src/service_dispatch.rs::outbound_router_response_into_result` | `OutboundResponse::Error`→`RuntimeError::ProviderUnavailable {reason:error.message}` | 原class/payload/correlation第一次永久丢失，并按message生成新local platform cause | 必须停止分类；未来只消费W2-W交来的typed fixed carrier，generic error应Protocol/fail closed |
| `runtime/eval/src/error.rs::user_exception_payload` | uncaught local exception→generic code `UnhandledServiceError` | payload被压平，只余trace/error id；diagnostic wrapper还可能追加本地source frame | 不是service response出口；canonical fixed cause必须在到达这里前被提取 |

当前caller侧两条“看似能catch”的真实路径都不是合格import：

- provider `UserException`沿Rust `Err`原样到caller，`promote_platform_error_at_call`看到已有exception后直接返回，
  `eval_program_catch`可能因assembly-global `TypeAddr`相等而catch；但payload handle仍属provider heap，
  correlation/stack也没有逐跳重建；
- 非`UserException` platform error由caller call-site promotion新建local exception；它能catch，却没有先形成
  `PlatformError`，也无法跨下一service保持原`errorId`。

## 已接入模型与真实入口

| 模型 | 已接入production | 仍仅模型/fixture或缺失 |
| --- | --- | --- |
| `ServiceErrorTypeIndex` | loader读取exact index/closure；linker建双向表；`AssemblyExecutionImage`持有；shared image保留schema index和完整records | eval/service response没有consumer；caller-linked候选选择、export/import均未实现 |
| `RuntimeValueCarrier` | slot、heap sidecar、object/array/map、call/return、native plan、local throw/catch/rethrow、test effect均真实使用 | service call参数被转成bare `RuntimeValue`；provider error没有detached；跨heap clone会盲目保留local `TypeAddr`，不能用作service import翻译 |
| `RequestException` | local throw、platform promotion、catch、rethrow、exception heap节点均真实使用 | 没有production `RequestException::opaque`或remote import；现有cause不能同时持raw envelope与local value |
| `ServiceErrorEnvelope`/`OpaqueServiceError` | strict serde、correlation校验、exact input bytes保留已有model单测 | runtime其余production零引用；没有export、import、stream、response carrier |
| `ExceptionStackFrame::RemoteBoundary` | DTO已冻结 | production零引用；provider context切换还继承caller `local_call_stack` |
| `PlatformBuiltinErrorIdentity` | capability/boundary/eval error projection、catch leaf/type-plan annotation与exact catch真实使用 | 没有`PlatformError`payload encoder/decoder；legacy response仍按generic code/message处理 |

platform registry只能识别冻结的有限集合。`std.resource.ResourceError`明确不在集合中，必须通过
`ServiceErrorTypeIndex`作为owner=`skiff.run/std`的普通`PublicTypedError`。`std.service.InternalError`
同样不是platform builtin；它是普通exact nominal value加fixed `InternalError` envelope语义。

## Canonical orchestrator owner与依赖方向

最小production owner：

```text
runtime/eval/src/assembly_execution/service_error_channel.rs
    CanonicalServiceErrorChannel
```

它应与`CanonicalServiceBoundaryPlan`相邻，但职责不同：

- boundary plan继续负责参数、成功返回值及capability的directional heap materialization；
- service error channel唯一负责failure分类、public/platform/Internal fixed envelope、opaque原样转发、
  caller-linked materialization、correlation和逐跳local stack；
- ordinary、async unary、stream terminal、ingress和service test effect只提供typed输入并调用同一API，
  不拥有分类分支。

当前Cargo依赖允许的方向是：

```text
artifact-model
      ↓
runtime/model
   ↙       ↘
boundary  linked-program
   ↓          ↑
capability   loader → linker（只在admission构建linked image）
      ↘      ↙
       runtime/eval
```

具体约束：

- `runtime/model`只继续依赖`artifact-model`，保存fixed DTO和request-local cause，不读取schema/index/heap
  orchestration；
- `runtime/boundary`只依赖`artifact-model`和`runtime/model`，提供schema codec，不依赖
  `linked-program`、`linker`或`eval`；
- `runtime/linked-program`只保存index DTO及linked image，不依赖boundary/eval；F298刻意不依赖
  `CatchIdentity`的边界继续保持；
- `runtime/linker`只在admission把loader/shared image转换成index；request执行不得反向依赖linker crate；
- `runtime/eval`当前已经依赖model、boundary、linked-program和capability-context，正好是第一个能够同时看到
  provider heap、request context、execution image、call site和schema closure的层；
- production `runtime/eval`不新增对`runtime/linker`的依赖；其Cargo中linker仍只允许dev-dependency用于fixture；
- boundary不回调eval，eval不把classifier下沉到boundary，因此不存在eval↔boundary cycle；
- boundary不读取linker/index，因此不存在boundary↔linker cycle；
- model不读取任何高层schema/runtime owner，因此不存在model反向依赖。

把owner放进boundary会迫使boundary依赖linked-program和request execution；放进linker会迫使admission层
依赖heap/eval；放进model则会让低层DTO依赖codec/schema。三者都会破坏上述方向。

## Exact public owner、schema、type plan与materialization

### Outbound public typed error

`export_provider_failure`应按以下exact顺序工作，不能按name、display、record shape、payload code或operation
contract猜测：

1. 输入是provider `RuntimeError`、仍存活的provider `RequestHeap`、provider
   `ProgramExecutionContext`、resolved operation/activation facts和caller boundary facts。先剥离
   diagnostic wrapper作为本地受限诊断输入；wire/envelope不携带source path/function/frame。
2. 若`RequestException`已经是imported fixed cause，立即返回其
   `OpaqueServiceError::encoded_bytes()`；禁止decode/re-encode、重新sanitize或生成新error id。
3. 对其余error先按typed variant而非payload字符串分流：
   - 新local `UserException`沿用其中已有correlation并继续步骤4；
   - 尚未materialize为`UserException`、但`catch_projection`给出有限
     `PlatformBuiltinErrorIdentity`的真实platform failure，只生成一次correlation并进入
     `PlatformError`；
   - loader/linker/artifact不变量损坏或malformed imported envelope保持
     `InvalidArtifact`/`Protocol`，不能被sanitize成普通Internal；
   - 其余provider-local runtime fault只创建一次新correlation并生成fixed Internal，不能把generic
     `RuntimeErrorPayload`直接当service response。
4. 对新local user cause读取`RuntimeValueCarrier::catch_identity()`：
   - `PlatformBuiltin`先进入有限`PlatformError`路径；
   - exact `std.service.InternalError`先进入fixed `InternalError`路径；
   - 其余`LocalExecution` declaration或named-union branch转换为
     `ServiceErrorExecutionKey`。
5. 先用linked declaration核对`type_arguments` arity。F298不接纳public generic row；合法local
   applied-generic cause不得拿裸`addr`误命中public row，而应进入nonclosed/Internal路径；伪造在
   non-generic declaration上的arguments是`InvalidArtifact`。只有zero-argument declaration才用
   `ServiceErrorExecutionKey::Declaration {addr}`查询
   `AssemblyExecutionImage::service_error_types().by_execution`。named-union由enclosing
   `union_addr`和exact `NamedUnionBranchIdentity`对照linked declaration branches得到唯一
   `branch_index`，再查`NamedUnionBranch {union_addr,branch_index}`。
6. `ServiceErrorTypeLink`给出唯一
   `ServiceErrorPublicIdentity {package_id,stable_schema_key,package_schema_type_id}`、root
   `PackageSchemaTypeRecord`及declaration/branch context。owner来自actual `TypeAddr.unit`所指package
   code slot，可以是throwing service自己的Package，也可以是其dependency；不得改写成service owner。
7. 用row的code slot从
   `AssemblyExecutionImage::shared_packages().code_by_slot(...).schema_records()`取得完整closure，而不是
   operation contract admitted parameter/return records。
8. 由public identity构造exact
   `ContractTypeRef::PackageSchema {package_id,stable_schema_key,package_schema_type_id}`，调用
   `ServiceValuePlan::compile(&contract_type,&records)`。compile现有逻辑会再次校验owner/key/type id和
   referenced record closure。
9. 用`PayloadBoundaryKind::ServiceResponse`及provider heap编码actual value。record/representation选择
   root plan；named union必须传F298 row中的exact `branch_index`。
10. 成功后生成`ServiceErrorEnvelope::PublicTypedError`，沿用local exception的`traceId/errorId`，再严格
   serialize一次成为fixed bytes。

底层binary union codec已经写入/读取一个`u8` branch ordinal，但当前
`ServiceValuePlan::encode_binary`先调用shape matcher，再由codec按分支顺序试编码；
`decode_binary`返回bare `RuntimeValue`并丢弃ordinal。最小boundary API补充应具有下列语义：

```text
encode_binary_selected(value, selection = Root | NamedUnionBranch(index), boundary, heap)
decode_binary_selected(bytes, boundary, heap)
    -> { value, selection = Root | NamedUnionBranch(index) }
```

名字可在实现中调整，但输入/输出事实不能减少。它必须验证selected index与compiled root plan一致，禁止
same-shape fallback；record/representation不能接受branch选择，union不能在没有exact选择时编码。

`PlatformError`不查`ServiceErrorTypeIndex`来决定类别。类别只能来自已经typed的
`PlatformBuiltinErrorIdentity`；R0还需在同一有限registry旁提供按enum key选择的canonical payload
plan/materializer，使local carrier或现有`catch_projection`payload得到同一bytes，并在inbound严格验证后
附着同一platform catch identity。当前`from_symbol/symbol`只有identity映射，不是payload codec；不得让
各lane用generic JSON/code各写一套。

### Inbound caller-linked materialization

1. `OpaqueServiceError::decode`先strict decode fixed envelope并保留完整原bytes。
2. `PublicTypedError`按完整三元组查询`ServiceErrorTypeIndex::by_public_identity`。候选必须再按当前caller
   activation的`implementation_package_build_id`及
   `SharedPackageLinkedImage::package_link_plan()` exact package edges过滤；assembly中“某处存在同
   package id/type”不等于caller链接该type。
3. 唯一caller-linked code slot提供root record及完整schema closure；同一public identity有多个build时
   不允许取first或按package id消歧，必须由exact build edge选中，歧义fail closed。
4. 同一`ServiceValuePlan`decode返回value和exact branch selection。record/representation选择其
   declaration row；union用decoded branch ordinal选择对应caller-linked branch row。
5. 用`EvalTypeProjection::plan_from_linked_nested_ref`为caller execution address创建本地
   `RuntimeTypePlan`，再用现有`runtime_ops::runtime_carrier_for_plan`在caller heap递归附着
   `LocalExecution`/named-union identity。不能把codec产生的`PackageSchema` identity直接拿去与caller
   catch leaf比较。
6. 最终`RequestException`同时保存上述caller-local carrier与步骤1的raw fixed envelope。local catch读取
   carrier identity；未捕获或rethrow到下一service时读取raw envelope。

### 生成一次InternalError所需最小输入

| 情况 | 已有输入 | 最小缺失输入/动作 | 结果 |
| --- | --- | --- | --- |
| private/non-nameable | local carrier identity、heap、correlation；index无row | model中的唯一fixed message常量；channel分类分支 | 一次`InternalError {message,traceId,errorId}` |
| source public但未形成合法SchemaClosed row（含local applied generic） | carrier有exact local identity，但没有可编码public row | 与private相同；不得因为api/display name自行构plan；若schema index反而声明了F298禁止的generic row，则在admission按B7失败 | 同上 |
| exact public plan但actual value encode失败 | index row、schema closure、plan、provider heap、correlation均已有 | 将`TypeMismatch`、ambiguous branch、cyclic/interface/codec actual-value失败收敛到一次Internal；本地保留诊断 | 同上 |
| index/record/export/owner/key/id不变量损坏 | loader/linker及strict inbound校验已有 | 不需要Internal输入 | `InvalidArtifact`或`Protocol`，禁止用Internal掩盖 |

Internal fixed message必须由一个model/eval常量owner产生，不能把encoder message、原type、字段、display或
payload泄露进去。生成Internal沿用原cause的trace/error id；没有新的随机/sequence id。

`std.service.InternalError`的真实source/API已存在于`std/service.skiff`和`std/api.yml`。materialize时应：

1. 在exact caller-linked `skiff.run/std` code slot的schema index中取
   `std.service.InternalError` entry及其type id；
2. 用`ServiceErrorTypeIndex`验证该public identity恰好对应一个caller-linked record declaration；
3. 在caller heap构造`message/traceId/errorId`三字段object，并用caller-linked local plan附着ordinary
   exact nominal identity；
4. 同时保留收到的`InternalError`raw envelope。若std row缺失/错配，是
   `InvalidArtifact`/`Protocol`，不能把Internal降级为opaque或platform字符串。

## Linked、unlinked、opaque与InternalError

现有cause：

```text
RequestExceptionCause
  = Local { value }
  | OpaqueService { error }
```

不能表达“linked inbound既有caller-local value，又有原始fixed bytes”。最小替换形状是：

```text
RequestExceptionCause
  = Local { value }
  | ImportedService {
      error: OpaqueServiceError,
      local_value: Option<RuntimeValueCarrier>
    }
```

- linked public/platform/Internal：`local_value = Some(...)`，可exact catch；
- unlinked valid public：`local_value = None`，任何local catch miss；
- 两者的outward export都只读`error.encoded_bytes()`；
- `RequestException::map_local_value`必须同时支持linked imported value的heap move，但绝不改raw bytes；
- `RequestException::local_catch_identity`只看可选local value；
- 新`fixed_service_error()` accessor只对`ImportedService`返回raw envelope；
- local throw/rethrow仍是`Local`，不受service codec影响。

Public inbound的结果矩阵：

| caller事实 | local表示 | catch | 未捕获再次出界 |
| --- | --- | --- | --- |
| exact build graph链接owner/type/branch | caller heap中的local carrier + raw envelope | exact nominal/union catch成功 | 原bytes、traceId、errorId不变 |
| assembly知道该identity，但caller graph未链接 | raw envelope，`local_value=None` | 必须miss | 原bytes透明转发 |
| known identity的owner/key/id发生冲突、branch/tag/encoded payload非法 | 无普通error value | `Protocol` | 不转成Internal或按shape修复 |
| malformed/unknown envelope variant、缺字段、空correlation | strict decode失败 | `Protocol` | 不转发 |

完全unlinked且语法上有效的public identity只能作为opaque处理；一旦它与当前admitted schema/index中的已知
owner/key/id发生部分冲突，就按B7 Protocol关闭，不能用“可能是另一个同名类型”回退。

Inbound `InternalError`不是opaque-only控制错误。它必须按上节materialize为普通
`std.service.InternalError` nominal value，因而caller可以像捕获普通error一样exact catch。它仍然是
`ImportedService` cause；若catch miss/rethrow，export先检查fixed cause并原样转发，不能根据materialized
record再次构造第二层Internal、再次sanitize或生成新error id。用户在本service新抛出的local
`std.service.InternalError`则由exact local identity识别，在第一次出界时创建一个fixed Internal envelope，
不能落入普通`PublicTypedError`。

每一跳stack规则：

- provider activation开始时必须清空继承来的`local_call_stack`，但继续共享request trace和error-id
  sequence；当前`with_runtime_assembly_target`只替换target，仍clone caller stack，是明确delta；
- provider初始throw/本地rethrow只保留本service local frames；
- import用caller当前local stack+call site创建新source/stack，再追加一帧只含
  `service_id/operation_id/error_id`的`RemoteBoundary`；
- 不导入callee `InstructionSourceSite`、path、function、diagnostic wrapper或exception heap node；
- same-request local rethrow复用同一source/stack；cross-service import总是新caller-local stack；
- ingress只export fixed envelope，不伪造一个外部caller-local exception。

## Consumer覆盖

| consumer | 是否能共用唯一入口 | 必须改动 | 明确negative/control path |
| --- | --- | --- | --- |
| ordinary unary | 能 | provider heap drop前`export_provider_failure`；dispatcher对internal origin调用`import_caller_failure` | success仍走directional plan |
| async unary | 能 | 与ordinary同一export/import，只由lane保留scheduling | caller/request cancellation select不伪造成provider response |
| server stream | 能 | provider task在heap存活时export fixed carrier；sink/consumer传typed service error，不传dynamic generic payload | consumer/request cancellation继续结束/取消stream，不做Internal分类 |
| ingress/WS ingress | 能 | 同dispatcher只执行export，把fixed Rust carrier交给后续W2-W | 不在eval构HTTP/WS error policy，不导入external caller |
| service `ContractOperation` test effect | 能 | setup payload/heap作为synthetic provider输入走相同export，再对caller走相同import | 不再deep-clone local TypeAddr到caller |
| package-direct test effect | 不应跨boundary | 保持现有local `RequestException`语义 | 不要求public schema/encode |
| host-boundary test effect | core可被后续consumer调用 | 当前runtime registry没有typed host-boundary kind；W2-R只冻结fixed carrier/API，W2-W host consumer必须带exact boundary kind调用 | 不按Package-shaped target、target string或message猜host/service |
| legacy remote service response | core可消费typed carrier | capability-context增加明确fixed service-error carrier；eval mapper只接受它 | generic `ResponseError`是protocol/control legacy，不再按message转ProviderUnavailable |

当前test-effect `TestEffectTarget`只有
`PackageCallable {package_build_id,callable_id}`和
`ContractOperation {operation_id,expected_protocol_identity}`，没有host-boundary discriminator。补充exact
typed fact是既定T2语义的实现输入，不需要新的用户设计选择；在fact出现前必须fail closed。

## Duplicate/legacy owner清单

以下owner必须被删除、限制为local/control用途，或改成唯一channel的typed carrier consumer：

1. `CanonicalServiceBoundaryPlan::materialize_provider_error`的原样`Err(error)`占位；它是当前ordinary/async
   共同旁路。
2. `ProgramExecutionContext::with_runtime_assembly_target`继承caller `local_call_stack`；provider
   activation需要明确的新stack scope。
3. `async_stream_cancel::run_provider_stream`直接
   `StreamRuntimeError::producer(error)`；它在provider heap drop前未固定service error。
4. `StreamRuntimeError::Producer(Box<dyn WirePayload>)`及
   `materialize_stream_runtime_error`的dynamic downcast只能继续服务local/general stream error，不能承载
   canonical service failure。
5. `RequestHeapOwnedStreamError`盲目clone本地carrier identity；它是local stream owner，不是service
   export/import。
6. `RuntimeTestEffectRegistry::materialize_local_test_throw`把setup heap carrier直接deep-clone进caller；
   service target必须停止使用，package target可保留。
7. `deep_clone_runtime_value_carrier_between_heaps`原样复制local `TypeAddr`；任何service boundary调用都是
   heap/identity隔离旁路。
8. service call参数/return的`.into_value()`丢根carrier identity；成功值可按contract重建，error不能靠
   static return plan重建。channel必须从`RequestException`actual carrier读取identity。
9. `ServiceValuePlan::{encode_binary,decode_binary}`对named union只保留shape结果；必须增加selected branch
   API，禁止same-shape first-match。
10. `runtime/eval/src/error.rs::user_exception_payload`的`UnhandledServiceError`只能作为非service
    top-level generic diagnostic fallback；canonical response不得经过它。
11. `runtime/capability-context/src/response.rs::ResponseError`和
    `OutboundResponse::Error`没有fixed bytes；不能承载canonical service error。
12. `runtime/eval/src/service_dispatch.rs::outbound_router_response_into_result`按
    `error.message`统一生成ProviderUnavailable，是明确legacy classifier，必须改成typed carrier
    pass-through或generic Protocol。
13. eval和boundary各自的`decode_target_error_code`、各crate `RuntimeErrorPayload.code`以及
    `PlatformBuiltinErrorIdentity::from_symbol`只能用于已有finite platform producer/catch projection；
    service channel不得重新从code/message反推identity。
14. `runtime_error_from_wire_payload`/generic `WirePayload::as_any`不能成为fixed envelope decoder。
15. `detached_error`只是effect guarantee/admission事实，不是response carrier或error classifier。
16. platform registry是合法唯一有限owner；不得复制allowlist到ordinary、stream、host、router或
    TypeScript。尤其不能把`std.resource.ResourceError`加入platform分支。

## 建议DAG

```text
F298 + F299 + F305 + F316
              │
              ▼
R0  Canonical service-error core checkpoint
      ├──────────────┬──────────────┐
      ▼              ▼              ▼
R1 ordinary/ingress  R2 async/stream/cancel  R3 service test effect
      └──────────────┴──────────────┘
                     ▼
R4 W2-R convergence probes（test-only）
                     │
                     ├──解除A5-runtime-channel preacceptance
                     └──向W2-W交付fixed Rust carrier/API
```

parallel任务只有R1/R2/R3；其production和test写入面互不重叠。R0在fan-out前冻结API，R4不写
production。不得把以下范围加入任一节点：
`runtime/request/**`、`runtime/transport/**`、`runtime/host/**`、`router/**`、`telemetry/**`。

### R0：Canonical service-error core checkpoint

- blocked-by：F298、F299、F305、F316已整合；F318不阻塞record/union/platform/Internal核心，只阻塞
  representation real-eval probe。
- production写入范围：
  - `runtime/model/src/service_error.rs`
  - `runtime/boundary/src/service_value_plan.rs`
  - `runtime/eval/src/assembly_execution/service_error_channel.rs`（新）
  - `runtime/eval/src/assembly_execution/mod.rs`
  - `runtime/eval/src/error.rs`
  - `runtime/eval/src/exceptions.rs`
  - `runtime/eval/src/program_execution.rs`
- test写入范围：
  - `runtime/model/src/service_error.rs`内单测
  - `runtime/boundary/src/service_value_plan_tests.rs`
  - `runtime/eval/src/assembly_execution/service_error_channel/tests.rs`（新）
  - 上述eval文件的现有inline test module
- 交付：
  - `ImportedService {raw envelope, optional local carrier}`cause；
  - branch-aware service value codec；
  - 唯一export/import/opaque-forward orchestrator；
  - exact caller build graph选择、public/platform/Internal materialization；
  - fixed Rust `RuntimeError` handoff、provider stack scope、per-hop import stack。
- 最小正向探针：
  - pure B1/B2 record与named-union exact owner/branch encode/decode；
  - B3 linked/unlinked materialization和raw-byte forward；
  - B4/B5/B6一次Internal；
  - B8/B8a platform-vs-Resource；
  - B9 imported Internal raw forward；
  - S2 local rethrow不变、remote import新stack。
- 最小负向探针：
  - owner/key/type id/record冲突、branch ordinal越界、same-shape错误分支、malformed envelope→Protocol；
  - opaque catch miss；private payload/encoder message/source frame不出现在fixed bytes；
  - caller graph中存在另一build但未链接时不能误materialize。
- 解除：R1、R2、R3；向W2-W提供唯一fixed carrier/API。
- 证据失效边界：
  - 修改`ServiceErrorEnvelope`、`RequestExceptionCause`、index lookup语义、branch-aware codec或caller graph
    selection时，R0全部probe及R1–R4 evidence失效；
  - F318落地只重跑representation-valued B1/B2/B4/B6，不推翻owner/DAG。

### R1：ordinary、dispatcher与ingress consumer

- blocked-by：R0。
- production写入范围：
  - `runtime/eval/src/assembly_execution/boundary_materialization.rs`
  - `runtime/eval/src/assembly_execution/ordinary.rs`
  - `runtime/eval/src/assembly_execution/ingress.rs`
  - `runtime/eval/src/assembly_execution/websocket_ingress.rs`
- test写入范围：
  - `runtime/eval/src/assembly_execution/boundary_materialization/tests.rs`
  - `runtime/eval/src/assembly_execution/ordinary/tests.rs`
  - `runtime/eval/src/assembly_execution/ordinary/test_runtime.rs`
  - `ingress.rs`及`websocket_ingress.rs`内co-located test module
- 交付：
  - provider heap drop前export；
  - internal origin在central dispatcher import到caller；ingress origin只上交fixed carrier；
  - provider activation stack reset；
  - 删除共同boundary passthrough占位，不在lane复制classifier。
- 最小正向探针：ordinary/ingress真实B1、B2、B4、B8、B9；三跳B3；S1/S2逐跳stack。
- 最小负向探针：wrong owner/key/id→Protocol；provider heap drop后无handle访问；ingress fixed bytes不含
  callee source；external ingress不创建caller-local exception。
- 解除：ordinary/ingress A5子门及W2-W ingress handoff。
- 证据失效边界：仅ordinary dispatcher/materialization/ingress变更使R1 lane evidence失效；不使R2
  stream scheduling或R3 test-effect evidence自动失效。R0 shared API变更则全部失效。

### R2：async unary、stream、cancel与service response carrier

- blocked-by：R0。
- production写入范围：
  - `runtime/eval/src/assembly_execution/async_stream_cancel.rs`
  - `runtime/eval/src/program_stream.rs`
  - `runtime/capability-context/src/stream.rs`
  - `runtime/capability-context/src/response.rs`
  - `runtime/capability-context/src/outbound_response.rs`
  - `runtime/eval/src/service_dispatch.rs`
- test写入范围：只限上述文件inline/co-located tests及
  `runtime/capability-context/src/lib.rs`中service-response/stream focused tests。
- 交付：
  - async unary provider terminal与ordinary调用同一export/import；
  - server-stream provider heap存活时export，stream carrier持typed fixed error；
  - consumer import不再依赖dynamic code/downcast；
  - cancellation control与provider error exact分流；
  - capability-context提供明确fixed service-error variant；
  - legacy generic `ResponseError`不再按message转ProviderUnavailable，未迁移producer只得到Protocol。
- 最小正向探针：async/stream B1、B3、B6、B8、B9及S1/S2；typed legacy seam保持exact bytes。
- 最小负向探针：consumer/request cancellation不生成Internal/Platform response；provider heap销毁后
  stream error仍可import；generic `ResponseError`不能被分类成typed error；opaque stream hop不
  decode/re-encode。
- 解除：async/stream/cancel A5子门；W2-W response.error producer/host consumer。
- 证据失效边界：stream carrier、terminal ordering或capability response enum变更只使R2及W2-W handoff
  evidence失效；ordinary/ingress和test-effect语义仍需各自证据。R0 shared API变更则全部失效。

### R3：service inline/test-effect consumer

- blocked-by：R0。
- production写入范围：
  - `runtime/eval/src/test_effect_registry.rs`
  - `runtime/eval/src/eval_context.rs`
- test写入范围：
  - `runtime/eval/src/test_effect_registry.rs`内单测
  - `runtime/eval/src/assembly_execution/ordinary/tests/source_inline_effect_e2e.rs`
- 交付：
  - `ContractOperation` throw不再直接clone到caller；以setup heap/actual carrier作为synthetic provider
    输入走R0 export，再从同一fixed carrier走caller import；
  - exact protocol/operation target保留；
  - `PackageCallable` throw继续走local request exception；
  - 缺exact host-boundary kind时fail closed并向W2-W暴露typed调用要求，不修改host。
- 最小正向探针：T2 service effect分别覆盖public、private、encode failure、opaque forward和per-hop
  stack。
- 最小负向探针：package-direct T1仍不调用service encoder；service effect不保留setup heap handle或
  setup local `TypeAddr`；Package-shaped/target string不能假装host boundary。
- 解除：T2的W2-R service半边；W2-W host-boundary test-effect consumer。
- 证据失效边界：test-effect target/outcome或dispatch ordering变更只使T2 evidence失效；不替代真实
  ordinary/stream probes。R0 shared API变更则全部失效。

### R4：W2-R convergence probes

- blocked-by：R1、R2、R3；representation cases另等F318。
- production写入范围：无。
- test写入范围：新建
  `runtime/eval/tests/service_error_channel.rs`，只使用真实loader/linker/execution image和runtime入口；
  不回写R0–R3 co-located fixture。
- 交付：在不进入host/transport/router的前提下汇合B1–B9（含B8a）、S1–S2、T2 service半边及各lane
  negative；记录host-boundary/W1仍由W2-W完成。
- 最小正向探针：ordinary、async、stream、ingress、service effect各至少一个public和一个Internal；
  三跳B3/B9；platform/Resource分流；逐跳stack。
- 最小负向探针：所有identity mutation/opaque catch miss/heap isolation/cancel control/generic legacy
  error；wire bytes不含callee source/private payload。
- 解除：A5-runtime-channel preacceptance候选及W2-W正式开工。
- 证据失效边界：任一R0 shared语义变更使全套失效；R1/R2/R3只使其对应lane和跨lane组合失效；
  后续W2-W只新增外部frame/host/router证据，不应改变已冻结的in-process export/import bytes及identity。

## 最早风险探针

当前HEAD没有canonical orchestrator，因此B1–B9、S1–S2、T2均不能通过真实service channel端到端执行；
已有model/index/local-exception单测只能证明输入DTO和局部不变量。最早便宜子集应按下表运行，不能等到
full workspace gate才发现heap或identity错误：

| 最早节点 | 立即可跑的子集 | 重点negative |
| --- | --- | --- |
| R0 | pure B1、B2、B3、B4、B5、B6、B7、B8、B8a、B9及S2 model/import | branch ordinal、same-shape、wrong build、raw bytes、Protocol-vs-Internal、opaque catch miss |
| R1 | ordinary/ingress B1/B2/B3/B4/B7/B8/B9，S1/S2 | provider heap drop、callee frame泄露、ingress误import |
| R2 | async/stream B1/B3/B6/B8/B9，S1/S2；cancel negative | cancellation误分类、dynamic generic payload、stream heap丢失、legacy message分类 |
| R3 | T2 service的public/private/encode/opaque/per-hop；T1 negative | setup→caller直接clone、Package-shaped target、package-local被wire化 |
| R4 | B1–B9（含B8a）×lane汇合、三跳S1/S2、T2 service | 任一lane自有classifier、原bytes变化、trace/error id变化 |

F318未整合前，R0可用明确构造的representation carrier做codec/index unit probe，但任何依赖真实
`LinkedExprIr::RepresentationWrap` eval的B场景都必须标为blocked；不得据此宣告service error owner失败，
也不得在本任务重做representation constructor。

## 设计缺口

无新增设计决策。

以下都是父节点已冻结语义下的实现delta，不需要用户选择：

- `ImportedService` cause的具体Rust enum/field命名；
- branch-aware `ServiceValuePlan`方法和selection DTO命名；
- fixed Internal message常量的具体内部owner；
- fixed envelope在W2-W response.error header/binary payload中的物理布局；
- host-boundary test effect exact kind在后续W2-W DTO中的字段名。

它们不得被用来引入optional legacy field、dual write、shape/name/code fallback或第二classifier。

## 审计约束与证据

- 已完整读取本任务、五个直接父结果及适用的workspace/repository instructions；未因不存在语义冲突继续
  向上扩读权威设计；
- 已按任务要求反搜并归类
  `ServiceErrorEnvelope|ServiceErrorTypeIndex|RequestException|RuntimeValueCarrier`、
  `UnhandledServiceError|RuntimeErrorPayload|response.error|detached_error`和
  `InternalError|ProviderUnavailableError|ProtocolError`；
- 未修改production、测试或权威设计；唯一写入是本result；
- 未运行cargo、workspace、stable、live或chat smoke；
- 未push，不承接实现。
