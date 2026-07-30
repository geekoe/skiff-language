# P5-F349 Public generic boundary availability audit result

状态：`Completed read-only / READY_TO_IMPLEMENT`。

审计基线：

- commit：`6fe25aa1c2545d76f63e96b0261516cfdc288e99`
- tree：`a458352384a28a055103ae17f617724d4026077f`
- branch：`codex/p5-f349-public-generic-audit`

本审计没有运行workspace、stable instance或live链路，没有修改production、test、std、corpus或
lockfile。

## 结论

最小语义是：

> generic declaration可以是`api.yml` public、进入`PackageLocalAbi`并由精确
> implementation link发布/导入；它及依赖它的shape在当前generation不进入
> `PackageSchema`。引用该shape的service-call callable使用现有typed
> `Unavailable(UnsupportedBoundaryType)`，不得把无关Package整体升级为错误。

这不需要新的artifact字段、wire状态或exact std symbol特例。现有artifact已经把package linkage与
service schema分成独立域：

| 表面 | generic declaration / fully applied nominal |
| --- | --- |
| `api.yml` public binding | 可公开、可命名 |
| `PackageLocalAbi.public_symbols` + `implementation_links.types` | 可保真承载、发布和导入 |
| 普通package source type resolution / lowering | 可按ordered arguments、exact owner使用 |
| 当前generation `PackageSchemaIndex` / records | 不可用；应为零记录，而不是整包错误 |
| service-call operation | 结构化`Unavailable`，不写入`ServiceContract.operations` |
| external ingress handler typing | 由compiler platform/prelude source与linked handler signature提供，不以这些generic records为前提 |

## 唯一first-loss owner

唯一最早把任意public generic declaration升级为整包错误的production owner是：

```text
compiler/projection/src/package_artifact/projection.rs
  project_compiled_package_artifact
    -> project_package_exports

compiler/projection/src/package_artifact/api_exports.rs
  project_package_exports
    -> export_bindings().public_symbols()
    -> 每个non-function public binding
    -> collect_package_api_symbol_abi_violations(
         ...,
         PackageBoundaryKind::PackageSchema,
       )
    -> collect_package_exported_type_binding_abi_violations
    -> generic declaration / schema-applied nominal violation
    -> ProjectionError::InvalidPackageArtifact
```

`project_compiled_package_artifact`先调用`project_package_exports`，之后才调用
`project_package_schema`。因此package schema尚未开始生成，公开generic已经被聚合成整包
`InvalidPackageArtifact`；这就是F301/F302当前production first loss。

`compiler/projection/src/package_artifact/schema.rs::project_package_schema`还有第二层同类拒绝：

- candidate有non-empty `type_params`时立即返回错误；
- descriptor/interface method含`AppliedNominal`时立即返回错误。

它是解除first loss后必然命中的冗余后置拒绝，不是first-loss owner。F301引入的
`PackageBoundaryKind`本身区分了`PackageLinkEntry`与`PackageSchema`，但
`project_package_exports`错误地把所有non-callable public binding都预判成必须拥有
PackageSchema，混淆了“公开链接能力”和“service schema能力”。

## 现有Local ABI、link、import与identity能力

下游机制已经具备所需表达力；当前只是被上述整包拒绝遮挡，尚不能完成真实public generic artifact的
端到端发布。

### 发布与链接

- `artifact-model/src/package_artifact.rs`中的`PackageLocalAbiSymbol::Type`保留
  `descriptor`、`type_params`、declaration kind与interface methods。
- `PackageArtifact`分别保存`package_local_abi`、`implementation_links`、
  `package_schema_index`和`package_schema_type_records`，没有要求四者同集合。
- `compiler/projection/src/package_artifact/callables/surface.rs`把public type export完整写入
  `PackageLocalAbi.public_symbols`。
- `compiler/projection/src/package_artifact/callables/mod.rs`和`normalization.rs`保留generic
  declaration、`AppliedNominal` wrapper、ordered arguments和exact `PackageSymbol` owner；
  implementation type与public link不会把它压平为anonymous shape。

因此“存在于Local ABI/link、缺席于PackageSchema”已经是可验证的typed artifact状态，不需要增加
`PackageTypeAvailability`一类新DTO。

### dependency import与source使用

`compiler/source/src/type_resolution_model.rs`的artifact package type indexing：

1. public dependency只从`package_local_abi.public_symbols`取类型事实；
2. 要求存在精确`implementation_links.types`条目；
3. 校验descriptor、`type_params`和interface methods一致；
4. 按声明参数检查generic arity；
5. 构造以exact package owner为base、保留ordered arguments的
   `TypeRefIr::AppliedNominal`。

普通package dependency的声明导入、source type checking和lowering不读取PackageSchema。
wrong arity、unknown declaration、不能作为applied base的interface/actor/external declaration仍在source
入口fail closed。

### identity与tamper rejection

- artifact identity把Local ABI public/implementation symbols、type parameters、
  implementation links及schema内容分别纳入canonical projection；
- applied nominal的owner、argument值和argument顺序都会改变Local ABI/build identity；
- artifact validation在owner type-parameter scope内验证`TypeParam`，要求applied arguments非空并递归
  校验；
- Local ABI与implementation link的descriptor、type parameters或interface methods不一致会拒绝；
- applied nominal以`PackageSchema`为base在当前generation仍拒绝。

因此放开package publication不等于放松dependency或identity admission。

## 仍然依赖PackageSchema的路径

以下路径必须继续只消费当前generation可编码的closed schema：

- `compiler/projection/src/package_artifact/boundary/types.rs`中的service-call parameter、
  return、stream/callback及package nominal projection；
- `compiler/contract/src/projection.rs`中的ServiceContract operation roots、transitive record
  closure和`PackageTypeRequirement`；
- `compiler/input/src/contract_dependencies/reader.rs`中的contract dependency schema bundle、
  record owner/key、reachable closure、artifact/requirement ABI与build校验；
- public typed error envelope所引用的package nominal类型；
- 其它显式以PackageSchema作为serialization/persistence contract的consumer。

public typed error使用开放错误通道，不是operation signature中的closed throw set，必须单独分层：

- non-generic、public、schema-closed error继续生成精确PackageSchema record并进入service-error execution
  index；
- generic error或内部含generic/applied nominal的error shape只可作为Local ABI类型，不得获得
  `PublicTypedError` identity/record；
- 抛出后一类local error不会使operation signature本身Unavailable；runtime必须按现有规则降级为固定
  `std.service.InternalError`，不得泄漏原type identity或payload；
- 对一个本应schema-closed并已拥有public identity的error，缺execution index row属于invalid artifact，
  不能静默降级；
- forged/mismatched public error owner、stable key、type id、payload或closure继续fail closed。

不得为了generic publication删除、伪造或局部生成public error record。

`compiler/projection-input/src/lib.rs::ResolvedPackageSchema::new`当前拒绝canonical descriptor带
non-empty `type_params`的`GenericTypeRecord`，并验证exact index/record、owner/key、closure、ABI和build。
这些consumer-side admission必须原样保留。

## 最小production修复范围

修复只需要调整projection capability分层，不需要改artifact model、contract wire、runtime或std源码。

1. `compiler/projection/src/package_artifact/api_exports.rs`
   - non-function public export只做package link/Local ABI admissibility校验；
   - 不再无条件以`PackageBoundaryKind::PackageSchema`校验每个public binding；
   - callable签名仍保留现有PackageLinkEntry验证和完整generic信息。
   - DB key/field等真正的persistent payload仍保留现有
     `PackageBoundaryKind::PersistentSchema`校验。
2. `compiler/projection/src/package_artifact/schema.rs`
   - generic declaration、含`AppliedNominal`/free `TypeParam`的shape不再返回整包错误；
   - 它们成为schema-ineligible candidate，不写index/ref/record；
   - eligibility必须递归：non-generic owner若字段、representation、union branch或interface method引用
     generic/applied nominal，也必须整体缺席，不能留下dangling或partial record；
   - package-symbol closure检查必须同时检查target declaration的`type_params`，不能只按path找到
     definition就视为eligible。
3. service boundary与consumer strict admission
   - 保留`AppliedNominal`/`TypeParam`到`UnsupportedBoundaryType`的现有投影；
   - 保留`ResolvedPackageSchema`、contract dependency与artifact identity的strict validation。
4. H35 external-ingress separation的相邻consumer迁移
   - `schema.rs::is_boundary_builtin`和
     `boundary/types.rs::validate_boundary_builtin`当前仍把
     `WebSocketIngressEvent<T>`/`WebSocketConnectResult<T>`接纳为通用service-call builtin；
   - `boundary/types.rs::project_boundary_operation_contract`还按WebSocket event参数改变通用operation
     cancellation contract；
   - 这些是旧ServiceContract-owned ingress的残留，H35实现时必须从通用service-call projection移出，
     由专用typed ingress projection消费linked signature；
   - source/prelude对四个platform generic的识别必须保留，它服务handler语言编译，不是
     PackageSchema/service-call许可。

不得在public export/schema-candidate的generic filtering predicate里添加
`std.websocket.WebSocketConnection`、`WebSocketReceiveEvent`、
`WebSocketIngressEvent`或`WebSocketConnectResult`的exact-name分支。任意owner、任意名称的public
generic declaration都应获得相同Local ABI可用、PackageSchema不可用语义。

## service-call structured Unavailable

现有状态机已经闭合所需语义：

```text
public callable
  -> BoundaryCallableProjection::Unavailable {
       reasons: [BoundaryUnavailableReason::UnsupportedBoundaryType]
     }
  -> ServiceApiFunctionStatus::Unavailable { reasons }
  -> 不进入ServiceContract.operations
```

`compiler/projection/src/package_artifact/boundary/types.rs`已经把
`TypeRefIr::AppliedNominal`和`TypeParam`投影为`UnsupportedBoundaryType`；
`compiler/projection/src/package_artifact/boundary/mod.rs`聚合为callable-level
`Unavailable`；`compiler/contract/src/projection.rs`只把`Available` callable写入contract。

若Package中还有其它Available public callable，contract正常生成且只包含它们。若全部callable都
Unavailable，contract definition的“至少一个operation”规则继续fail closed；不能生成一个缺类型或
伪closed的operation。这是contract产物不可生成，不应倒退为generic declaration导致Package不可发布。

## `std.websocket`四个generic platform types

`std/websocket.skiff`声明并由`std/api.yml`公开：

- `WebSocketConnection<Context>`
- `WebSocketReceiveEvent<Context>`
- `WebSocketIngressEvent<Context>`
- `WebSocketConnectResult<Context>`

修复后四者都应：

- 存在于`PackageLocalAbi.public_symbols`；
- 存在精确`implementation_links.types`；
- 保留一个ordered `Context` type parameter及完整declaration shape；
- 在`PackageSchemaIndex.types`中对应条目数为零；
- 在`package_schema_type_records`中对应record数为零；
- 不产生指向它们的schema ref或partial closure。

这四个类型不是PackageSchema特权类型。它们作为platform type的来源是：

- `CompilerPlatformSources`加载official package root、`std/api.yml`与canonical std source；
- `compiler/source/src/prelude_registry/loading.rs`从`std/websocket.skiff`登记声明、字段和type
  parameters；
- source resolution/type checking通过prelude registry解析并检查generic arity；
- expression assignability从registry declaration展开object-literal target并替换type parameters；
- lowering把已知WebSocket ABI generic保留为带arguments的compiler builtin/type ref；
- external ingress runtime codec按H35设计来自精确linked handler signature和canonical external-ingress
  shape，而不是上述四个PackageSchema records。

当前代码还有三个必须与publication first loss分开归因的exact platform seam：

- `compiler/source/src/type_resolution_model.rs::is_std_abi_generic_type_symbol`把四者解析为compiler
  platform builtin；这是handler source typing所需事实，应保留；
- `schema.rs::is_boundary_builtin`与
  `boundary/types.rs::validate_boundary_builtin`只把
  `WebSocketIngressEvent`/`WebSocketConnectResult`接纳进通用service boundary；
- `artifact-model/src/websocket_ingress.rs::WebSocketContractBuiltin`仍把Event/Result描述成
  ServiceContract builtin vocabulary。

后两项是H35所纠正的旧external-ingress/ServiceContract耦合，必须由F347及共享model/identity checkpoint
迁出通用service-call surface；它们不能成为F349绕过generic schema规则的依据。
`WebSocketConnection`/`WebSocketReceiveEvent`同样不能跨service-call。generic publication修复本身只解除
无关Package整包失败，不能宣称旧ingress surface已经闭合。

## 必须补齐的负例

| 负例 | 必须结果 |
| --- | --- |
| 任意名称/owner的public generic record、representation、named union | Local ABI/link存在；schema index/record为零 |
| non-generic public owner的任意字段/branch/method引用generic或applied nominal | owner整体schema-ineligible；无partial/dangling closure |
| public callable参数、返回、stream/callback引用generic/applied nominal | exact signature仍在Local ABI；service API为`Unavailable(UnsupportedBoundaryType)`；contract无该operation |
| 抛出public generic/non-schema-closed local error | operation availability不因open error channel改变；不得生成`PublicTypedError`，输出固定`InternalError` |
| schema-closed public error缺execution index row或identity/record被篡改 | invalid artifact / exact validation failure；不得静默降级 |
| dependency import使用wrong arity、unknown declaration或非法applied base | source resolution拒绝 |
| 篡改Local ABI与implementation link的descriptor/type params/methods | artifact validation拒绝 |
| 改变applied nominal argument、顺序或package owner | Local ABI/build identity不同；旧expectation拒绝 |
| 手工注入generic PackageSchema record | `ResolvedPackageSchema::GenericTypeRecord`拒绝 |
| 手工注入applied `PackageSchema` base | current-generation admission拒绝 |
| schema record缺失、额外、owner/key错误或ABI/build不匹配 | package/contract dependency reader拒绝 |
| non-generic public typed error record被删、改或伪造 | exact closure/identity校验拒绝 |
| 四个std generic类型与任意普通package generic类型对照 | 相同publication/schema语义；generic filtering无exact-symbol分支 |
| `api.yml` callable直接使用WebSocket Event/Result platform builtin | H35最终service-call projection为`Unavailable(UnsupportedBoundaryType)`；专用ingress projection独立判定 |

还需保留已有“private generic + fully applied public callable”测试，并新增真实artifact dependency import的
public generic正例，避免只覆盖legacy/in-memory type facts。

## generation与golden判断

不需要：

- 新`PackageArtifact`、`PackageSchema`或`ServiceContract`generation；
- 新wire field、availability enum或identity prefix；
- 允许generic `PackageSchemaTypeRecord`；
- 允许applied nominal以`PackageSchema`为base。

原因是修复只改变现有字段之间的成员资格：Local ABI/link保留类型，PackageSchema集合不包含它，
service projection使用已有Unavailable reason。

需要从修复后的canonical projection重新生成并审查所有受内容影响的identity/golden：

- std `PackageLocalAbiIdentity`与`PackageBuildId`；
- std `PackageSchemaIndexIdentity`及schema record bundle；
- 引用新Local ABI expectation的`PackageRequirement`和下游artifact build identity；
- package publication/authoring与artifact identity exact golden；
- 只有operation availability或contract surface实际改变时才更新相应
  `ServiceProtocolIdentity`/contract golden。

std source snapshot未改变时prelude identity原则上不应因本修复改变，但必须由测试验证，不能手工假设。
这是content identity重算，不是generation bump；旧identity、旧schema records或stale store pointer都不得
继续被接受。

## F302及eval闭合条件

F302不能在只修改golden、使用旧std artifact或绕过publication guard后判定通过。重跑前必须先证明：

1. canonical std从干净输入重新发布成功；
2. 四个WebSocket generic在Local ABI/link中完整存在；
3. 四者的PackageSchema index/record精确为零；
4. 非generic std public/error schema（例如`std.service.InternalError`）仍存在并通过exact closure；
5. 新artifact/schema/build identities与store/pointer均来自本次projection；
6. generic dependency import、service-call Unavailable和所有tamper/forged-schema负例通过。

然后原样重跑F302同一combined probe selectors，不能减少枚举或用替代fixture：

- `file_ir_execution_type_representation`
- `package_imports`
- `test_artifact_identity`

并原样重跑被同一canonical std publication first loss遮挡的runtime/eval测试：

- `representation_combined_probe::compiler_wrap_continues_through_file_ir_linking_and_eval`
- `source_inline_effect_e2e::source_inline_service_effect_sequence_typed_throw_is_caught_then_responds`
- `source_inline_effect_e2e::source_inline_compiler_owned_std_effect_replaces_the_exact_package_callable`

combined probe会经过production package compilation；两个source-inline测试都先seed canonical std。
只有它们在fresh identities/store输入下越过std publication并到达各自原始representation、link/eval和
effect/error断言，才能关闭F302遮挡。runtime/eval本身没有由本审计发现的修复点。

F302通过只关闭generic publication造成的旧遮挡，不替代H35 external ingress验收。后者还必须在F347共享
projection/model迁移后证明：同一linked handler可由专用ingress projection接受，而同一类型不会因此进入
ServiceContract或改变`ServiceProtocolIdentity`。
