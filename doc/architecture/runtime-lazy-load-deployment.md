# Runtime Lazy-Load Deployment Model

## 本文负责 / 不负责

本文定义部署与版本解析的长期模型：**不可变产物 + 小指针表 + 注册目录 + runtime 懒加载**。
每个service release都由自己的exact pointer决定；不存在跨service的全局运行记录或切换协议。

本文不定义语言语法、YAML字段、artifact DTO细节或CLI拼写。Package / ServiceContract /
ServiceDeployment与可选release bundle的对象语义仍以
[`package-service-contract-deployment.md`](package-service-contract-deployment.md)为最高权威；本文拥有
registry immutable-write、release pointer、版本解析、runtime load与rollback机械契约。

## 四条不变式

1. **产物只增不换**。内容寻址的immutable record（file-ir、package artifact、service contract、
   service deployment，以及可选release bundle）一旦写入永不变更。公开artifact的相同内容产出相同全局
   identity；含protected config ref的deployment在同一store security domain内确定性重构建为同一buildId。
2. **唯一可变状态是一张小指针表**。`(profile, serviceId, version) → buildId`，单键原子更新；
   多个版本可以同时存在，也不聚合成跨service运行对象。
3. **runtime 无协调状态**。runtime 以 buildId 为唯一加载单位：内存中有就直接执行，没有就从
   配置的 artifact 目录懒加载；加载不到就是错误。runtime 不感知人类版本坐标，也不参与跨runtime切换协议。
4. **路由 fail closed**。请求携带人类坐标 `(serviceId, version)`；router 解析指针得到 buildId，
   只派发给"已加载该 buildId 或具备懒加载能力"的 runtime；解析不到、无候选或加载失败一律快速失败。

## 概念与身份

- **人类坐标**：`serviceId + version`。version 来自 `package.yml` 的 version 字段。前端/调用方
  携带 version，不携带 buildId。
- **buildId**：内容哈希（`skiff-package-build-*` / deployment artifact identity）。是 runtime
  唯一的加载单位，也是指针表的取值。
- **指针表**：`(profile, serviceId, version) → buildId`。复用 typed pointer store 的原子写机制
  （rename + lock），单键更新天然原子。rollback = 把键指回旧 buildId。
- **deployment 记录**：服务的一次发布产物，以不可替换的deployment-owned `BakedConfigPayloadRef`冻结
  protected配置，指向package闭包与file-ir，并保存service dependency contract slot；它不保存provider
  build或runtime address。
- **DeploymentExecutionImage**：一个 exact deployment `buildId` 的 immutable runtime image。它只闭合
  该deployment的package-direct code/type/const/capability。它是唯一运行执行单位；跨service provider在
  每次boundary invocation开始时另行解析并pin，不进入consumer image identity。

## Immutable store、pointer 与 audit

Registry分别存储PackageArtifact及其子记录、ServiceContract、ServiceDeployment与可选release bundle。
这些对象没有共同kind或共同父DTO。写入必须按canonical bytes计算identity，在目标目录写临时文件、完整
flush/rename后重读校验；record缺失、hash/owner不符或引用闭包不完整都fail closed。Reader不能观察半写入
record，也不能从source revision补齐runtime事实。

Runtime release唯一可变状态是：

```text
ReleasePointerKey(profile, serviceId, exactVersion) -> deployment buildId
```

Pointer更新是单键原子操作；实现可以使用同目录rename + lock或等价CAS。Publish必须先使目标deployment及其
闭包完整可读，再更新pointer。每次production变更保存append-only history，至少记录old/new target、actor、
时间、原因和toolchain/provenance receipt；audit不成为runtime fallback。Rollback只把同一key指回一个仍可
验证的旧buildId，不修改历史record。

Package coordinate pointer可以作为compiler/tooling解析依赖的独立typed index，但不能被Runtime用于选择
已构建image内的PackageArtifact。Source revision同样只服务可信重编译、审计与复现，不是Runtime加载对象。
Registry service或CLI可以拥有这些durable facts；Router/Runtime只消费已经materialize的immutable store与
release pointer，不反向调用registry业务service。

## Runtime 懒加载

runtime 收到一个待执行的 buildId：

1. 内存中已有该 buildId 的已验证 `DeploymentExecutionImage` → 直接执行。
2. 没有 → 进入该 buildId 的临界区（per-buildId 锁）：同 buildId 的并发请求在锁外等待；
   持锁者按内容寻址从配置的 artifact 目录读取 deployment 记录、package 闭包与 file-ir，先做
   pre-link structural validation，再 link relocation/constant heap，最后做 post-link semantic verification
   并构建 image。PackageArtifact closure中的`platformErrorProjectionRegistry`必须唯一一致，每个
   PackageArtifact又必须与自己的bytecode header以及Runtime binary的generated singleton exact-match；任一
   缺失、mixed fingerprint或mismatch都fail closed。完整 VM 契约见
   [`bytecode-vm.md`](bytecode-vm.md)。
3. 加载成功 → 注册进"已加载集合"，放行等待的请求。
4. 加载失败或超时（记录缺失、目录不可达、超时阈值）→ 等待的请求快速失败。

同一个 buildId 不得因load期间任何service pointer的取值不同而生成不同image。Service dependency slot只保存
`serviceId + exact version + expected protocol identity`；执行该slot时解析provider pointer，取得provider
buildId并加载/进入另一个image。一次invocation及其stream/callback pin该provider owner；pointer更新只影响
后续新invocation。不同service pointer彼此独立，读者可以观察到任意合法的新旧build组合。

已加载集合是runtime本地cache，可以增长，也可以按本地策略（如LRU）逐出未被owner pin的image；逐出后
回到同一懒加载路径。Cache变化不修改release pointer，也不要求其它replica同步。

runtime 的注册与能力通告：

- 注册内容 = 已加载 buildId 集合；
- 能力通告使用runtime-frame-v5的`runtime.capabilities`，其strict metadata除artifact root与lazy-load能力外，
  必须携带`capabilities.platformErrorProjectionRegistry` exact descriptor，供router判定“已加载”与“可加载但
  尚未加载”的候选；缺失descriptor或旧frame直接拒绝。
- Registry descriptor是Runtime binary/session incarnation authority。同一WebSocket session内的capabilities
  refresh可以重复相同exact descriptor，但冲突值必须终止session；更换fingerprint只能建立新的session
  incarnation，不能原地改写registration facts。

## Router 派发

请求解析：`(serviceId, version)` → 指针表 → strict routing view
`{ buildId, registryDescriptor }`。该typed authority来自exact deployment PackageArtifact closure；Router只读取
validated routing metadata，不解析Package executable。Closure内descriptor不唯一时不能生成routing view。

候选集：注册了该buildId且registry descriptor exact-match的runtime ∪ descriptor exact-match、具备懒加载能力
且共享同一artifact store的runtime。HTTP、WebSocket、Actor与task等每条Runtime execution route都使用这项
session admission，不能由各route owner另建fingerprint truth。无任何候选 → fail closed
（`no eligible runtime`）。派发后runtime按懒加载语义执行并最终复验三方authority，加载不到或mismatch即错误。

新版本上线路径：发布新 buildId → 更新指针（单键原子）→ 后续请求自然解析到新 buildId。
窗口期语义：新 buildId 尚未被任何 runtime 加载时，请求快速失败；可选地，router 可向能力者
发送 fire-and-forget 的预载提示（无 ack、无 pending、失败就重试）来收敛窗口，但不构成协议。

## 发布、版本与回滚

- **新增版本**（version 变化）：新 buildId + 新指针键，旧键不动，新旧版本并行存在。
- **同版本覆盖**（version 相同，唯一替换路径）：新 buildId + 单键指针更新。旧 buildId 的
  image 在已加载集合中保留；在途请求不受影响；新请求走新 buildId。
- **回滚**：指针指回旧 buildId。旧 buildId 大概率仍在 runtime 内存中，瞬时可用；不在则懒加载。
- **可选离线聚合**：`ReleaseBundle`只列出exact `ServiceDeploymentRef`与verification receipt ref/receipt，
  便于verify或promotion复现；bundle identity由这两组事实确定性计算。它不进入load、routing或request。

## 多 runtime

多个 runtime 实例（共享或各自持有 artifact store）是可互换的工人：各自独立懒加载、互不等待、
互不通知，零协调成本。新版本通过"收敛"而非"切换"扩散到全部实例。单 runtime 与多 runtime
没有任何语义差异——多 runtime 只是同一模型的水平复制。

Rolling registry upgrade允许不同whole-registry fingerprint的session并存，它们只处理descriptor匹配的build。
只要任一可路由release/artifact仍引用旧fingerprint，operator不得清退最后一个matching session；回收条件是已无
可路由build引用该descriptor，而不是新binary已经连接。跨fingerprint service call仍按service error channel的
unknown-entry opaque forwarding规则传播，不能因此放宽artifact/session admission。

## Artifact store 与 Runtime bootstrap

Router配置唯一规范化`artifactsPath`与`serviceDb.mongoUrl`。Runtime主动连接后，Router在dispatch前发送
一次连接级bootstrap，至少包含这两个事实和Runtime所需的HTTP response limit；同一连接内缺失、重复冲突或
变更都fail closed。Router与Runtime可以位于不同机器，但`artifactsPath`必须指向内容一致的共享immutable
store。它们不从源码、registry service、ambient environment或Runtime本地默认值重建这些事实。

Router只读取release pointer以及包含`{ buildId, registryDescriptor }`的strict routing metadata，不解析Package
executable。Runtime按buildId读取deployment闭包、protected config payload、PackageArtifact与bytecode并构造
image。`artifactsPath`、Mongo URL和HTTP
limit是operator topology，不进入PackageArtifact、ServiceContract、ServiceDeployment或build identity；
service代码也不能读取provider URL或物理database name。

## 配置归属

服务配置（API key、provider端点等）在发布时冻结到ServiceDeployment-owned `BakedConfigPayload`，随buildId
懒加载。Protected ref是store security domain内确定性的keyed identity：同domain同canonical payload得到同一
ref，不同domain得到不同ref，且不知道domain key的一方不能离线枚举候选secret。Ref进入deployment identity；
payload没有独立selector或发布lifecycle，缺失、替换、解密失败或校验失败都会使整个image load失败。Ref的
唯一算法owner、immutable-put与secret规则见
[`package-service-contract-deployment.md`](package-service-contract-deployment.md) §11；
authoring规则见[`../reference/config.md`](../reference/config.md)。

## 流水线接口

与部署流水线（publish / deploy / verify / rollback）对齐：

- **publish**：产出buildId + deployment记录（可选附带只含refs/receipts的`ReleaseBundle`），写入不可变store；
  在各自identity domain内对相同canonical输入幂等。
- **deploy**：更新指针（单键原子）+ 可选预载提示；目标环境（dev / 线上）只是不同的
  store / router / 指针表实例。
- **verify**：对exact deployment refs执行health / smoke，并可把结果写成receipt后聚合进bundle。
- **rollback**：指针指回旧 buildId。

watch 只是 deploy 的自动触发器，不拥有流程。

## 文档所有权

- 本文是release pointer、immutable store、lazy load、replica与rollback的唯一长期事实源。
- [`package-service-contract-deployment.md`](package-service-contract-deployment.md)拥有artifact对象、identity、
  ServiceContract和service/package boundary。
- [`managed-dev-watch.md`](managed-dev-watch.md)只拥有本地registry、fingerprint、重试与由watch管理的pointer
  ledger；它不得为多个service pointer增加共同事务。
- [`bytecode-vm.md`](bytecode-vm.md)拥有deployment image的decode/link/verify/execute内部契约。

## 开放问题

- 懒加载 image 的内存上限与逐出策略（本地策略，先不做）。
- 同版本覆盖的窗口期是否需要在线上默认开启预载提示。
