# Compiler / Runtime Shared Artifact Types

本文定义compiler、artifact identity、deployment、runtime和Router围绕canonical artifact DTO、identity与
linked overlay的长期边界。四类顶层对象及其语义以
[`package-service-contract-deployment.md`](package-service-contract-deployment.md)为最高权威；本文只规定
共享类型的owner与依赖方向。

Skiff尚未发布。`PackageUnit`、`ServiceUnit`、共同ABI unit、service assembly和`RuntimeProgram`不是目标态
类型，不保留reader、writer、alias或fallback。

## Owner 分层

```text
compiler typed Package facts
  -> artifact-model canonical DTOs
  -> artifact-identity canonical projections
  -> deployment / assembly projection
  -> runtime loader + linked overlays
```

Router位于Rust共享代码之外：

```text
canonical artifact JSON
  -> strict TypeScript reader
  -> language-neutral golden/parity fixtures
  -> Router routing snapshot
```

Router不得成为canonical identity owner；Rust也不得复制一套Router hash。若TypeScript无法直接复用Rust
实现，producer写入identity，Rust/TS readers按同一schema和golden严格重算或调用窄CLI边界验证。

## `skiff-artifact-model`

`skiff-artifact-model`拥有磁盘/wire canonical DTO：

- File IR units与refs；
- PackageArtifact、PackageLocalAbi、PackageSchema record/index；
- ServiceContract、ContractOperation与Package type requirements；
- gateway entry、adapter source/plan与GatewayEntryIdentity；
- ServiceDeployment、operation/gateway/dependency/config/state bindings；
- RuntimeAssembly、activation templates与global ingress bindings；
- 共享type refs、callable signatures、value plans、error carrier metadata和runtime requirements。

规则：

- DTO使用typed fields和closed enum；unknown fields默认fail closed。
- Raw/open payload只允许在明确记录的外部metadata扩展点。
- Artifact DTO不拥有compiler AST/lowering、runtime address、Router socket或deployment mutable pointer。
- Type/schema metadata描述linking与boundary，不定义普通`RuntimeValue`物理布局。
- 同一个事实只能有一个canonical DTO owner；consumer不得建立同形“方便副本”并独立演进。

## `skiff-artifact-identity`

`skiff-artifact-identity`拥有跨Rust子系统必须逐字节一致的：

- PackageArtifact、PackageLocalAbi、PackageSchema record/index identity；
- ServiceProtocolIdentity与ContractOperationId；
- GatewayEntryIdentity及entry-local external shape normalization；
- ServiceDeployment revision；
- RuntimeAssembly identity；
- 公用hash framing、canonical ordering和validation。

Identity函数消费typed DTO或专用typed projection，不通过“序列化整个对象后删字段”决定preimage。Identity
名字必须与实际domain一致；不得让implementation build进入ServiceProtocolIdentity，也不得让handler binding
进入只代表external protocol的GatewayEntryIdentity。

## Compiler 输出边界

唯一Package compile pipeline产出：

```text
CompiledPackage
  -> FileIrUnits
  -> PackageArtifact
  -> optional ServiceContract
  -> typed gateway entry projections
```

随后deployment projection消费精确typed artifacts和所选profile：

```text
PackageArtifact + ServiceContract + gateway projections + config bindings
  -> ServiceDeployment
```

Assembly projection消费操作面选择的精确root deployments及闭包：

```text
ServiceDeployments + exact contracts/packages
  -> RuntimeAssembly
```

这些roots在dev来自watch registry或显式service roots生成的deployment receipts，在production来自平台部署
状态。源码仓库不author `assembly.yml`；项目间关系仍由各自`package.yml`拥有。RuntimeAssembly是projection
产物，不是developer config。

Compiler内部stage不得把`serde_json::Value`当typed model副本。JSON只在YAML/JSON input trust boundary或最终
emission出现；进入语义阶段前必须parse成strict DTO，emission不得重新做name/type/effect分析。

## 四类顶层对象的边界

### PackageArtifact

PackageArtifact拥有代码和Package本地链接事实：

- File IR与implementation links；
- PackageCallableId及exact signatures；
- PackageLocalAbi中互斥的public/implementation source-call indexes；
- PackageSchema index refs；
- dependency/runtime requirements；
- callable effect、provenance与boundary availability。

Artifact identity owner按精确`PackageCallableId`把每条callable link唯一绑定到上述两个index之一，再以
该exact signature scope校验对应implementation executable target/kind。它不能用同executable、显示路径或
public-first fallback代替精确owner，也不能把implementation-only callable提升到public index；
implementation callable id必须与其`top-level:<sourcePath>` canonical owner一致。

它不含`serviceCallRoots`或任何`service.yml`选择；同一PackageArtifact可被不同service projection复用。

Runtime linking可以把这些refs投影成`FileAddr`、`TypeAddr`、`ExecutableAddr`与linked indexes，但不能把地址
写回artifact。

### ServiceContract

ServiceContract是code-free service-to-service API：

- 只含`service.yml.serviceCalls`按Package public path显式选择后生成的operations；
- 使用ContractOperationId和PackageSchemaTypeId closure；
- 不含PackageCallableId、handler、route、config、build或external ingress；
- 不声明operation-specific throw set。

Consumer compiler只读该contract及精确PackageSchema records，不读provider source或deployment。

### ServiceDeployment

ServiceDeployment绑定实现与环境：

- ContractOperationId -> PackageCallableId；
- gateway entry key -> GatewayEntryIdentity、exact handler/pre/guard和typed execution plan；
- external selector -> gateway entry key；
- dependency、config、state/resource和activation policy；
- 精确PackageArtifact与expected ServiceProtocolIdentity。

GatewayEntryIdentity只覆盖external protocol；具体callable/build/plan由deployment revision固定。

### RuntimeAssembly

RuntimeAssembly固定一个可执行闭包：

- root与resolved ServiceDeployment refs；
- resolved ServiceContract/PackageArtifact refs；
- linked program image refs；
- 每个ActivationContext的binding template；
- global ingress selector到gateway entry binding；
- assembly identity与generation所需事实。

Runtime不得在load时读取latest pointer或按service/package display name补provider。

## Runtime Linked Overlay

Runtime loader先严格验证完整artifact graph，linker才创建runtime-owned overlay，例如：

- `LinkedProgramImage`
- `LinkedFileUnit`
- `LinkedTypeRef`
- `LinkedCallTarget`
- `LinkedPackageCallableIndex`
- `LinkedContractOperation`
- `LinkedGatewayEntry`
- `ActivationContext`
- request/stream/connection generation pins

Overlay可丢弃执行不需要的compiler-only数据，但只能在显式conversion中完成。Overlay不能修改共享DTO，也不能
成为新的artifact writer。

Package direct call与service call使用不同linked target：

- Package call解析PackageCallableId并在当前linked program内直接执行。
- Service call解析dependency slot与ContractOperationId，切换provider ActivationContext并执行service
  boundary materialization；物理同进程也不能改走Package target。
- External ingress按selector命中LinkedGatewayEntry，直接进入目标activation的普通request执行，不伪造
  ContractOperationId。

## Package 类型的两个身份域

共享模型必须保持：

- Package Local ABI nominal identity：用于compiler type equality与同一linked program链接，可包含
  closure-only type；
- PackageSchemaTypeId：用于service boundary，要求唯一canonical public path和schema-closed descriptor。

ServiceContract只引用PackageSchemaTypeId。Runtime不得用File IR local index、display name、Local ABI
closure-only identity或shape equality恢复service wire type。

## Router 边界

Router只消费执行routing所需的strict view：

- RuntimeAssembly/deployment/ingress identity与generation；
- IngressSelector、gateway entry key与GatewayEntryIdentity；
- service protocol expectation和opaque request/response bytes；
- deadline、cancel、stream sequencing、WebSocket connection pin与telemetry metadata。

Router不解析Package executable、业务type descriptor或payload，不选择handler target，也不从
ContractOperationId合成external gateway identity。

## Generation 与严格读取

任何wire shape语义变化都必须更新对应generation marker，并由reader拒绝旧shape：

- 移除/重命名字段时无dual-read；
- required identity缺失或不匹配时fail closed；
- nested unknown fields同样拒绝；
- fixture/golden与producer/consumer在同一commit更新；
- generation bump不用于掩盖未定义的identity preimage。

## Verification

结构与测试必须证明：

- production没有旧Unit/RuntimeProgram/common aggregate reader或writer；
- compiler只有Package source pipeline；
- 四类顶层artifact没有共同kind/父DTO；
- identity只有一个canonical owner；
- Router与Rust通过共享fixtures保持parity，没有第二hash算法；
- raw JSON不穿过typed stage；
- runtime linked address不写回artifact；
- Package、service和external ingress三种call target不会互相fallback；
- private external handler/type不会被提升为ServiceContract或PackageSchema public type；
- malformed owner、identity、closure、binding和unknown field均在执行前fail closed。
