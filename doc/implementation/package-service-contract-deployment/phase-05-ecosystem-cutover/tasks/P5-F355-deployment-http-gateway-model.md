# P5-F355 Deployment HTTP gateway model

状态：Ready（C1 deployment checkpoint；依赖F351、F352、F354）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F351-gateway-artifact-model-identity-result.md`
- `P5-F352-service-call-root-selection-result.md`
- `P5-F354-strict-http-service-authoring-result.md`

父节点沿引用链连接唯一权威设计。本任务只实现父节点已经冻结的deployment artifact语义，不重新设计
HTTP authoring、handler signature、WebSocket业务消息路由或runtime执行。

## DAG位置与目标

把ServiceDeployment从旧的
`IngressSelector -> ContractOperationId`改为：

```text
GatewayEntryKey -> resolved deployment gateway entry
IngressSelector -> GatewayEntryKey
```

本leaf冻结deployment DTO、strict validation、projection和identity generation。Compiler把source
handler/pre/guard解析为exact callable、派生external schema和完整codec计划属于C2；runtime、Router和
test-runner执行新entry属于C3。

## 必须完成

### 1. 唯一deployment gateway DTO

在shared artifact model中定义并由`ServiceDeploymentInput`与`ServiceDeployment`共同消费：

- `GatewayAdapterPlan`
  - required closed `GatewayAdapterKind`；
  - ordered `Vec<GatewayAdapterArg>`，保存目标参数到typed adapter source的完整映射。
- resolved deployment gateway entry
  - required `GatewayEntryIdentity`；
  - required canonical `GatewayEntryProtocolSurface`；
  - required handler `PackageCallableId`；
  - optional pre与guard `PackageCallableId`；
  - required `GatewayAdapterPlan`。
- `gatewayEntries: BTreeMap<GatewayEntryKey, resolved entry>`。
- deployment ingress binding严格为
  `{ selector: IngressSelector, gatewayEntryKey: GatewayEntryKey }`，不得再有
  `contractOperationId`、source path、display operation或兼容union。

类型名可按现有模块命名风格调整，但上述字段、closed vocabulary和owner关系不能改变。DTO必须
`deny_unknown_fields`；`gatewayEntries`反序列化必须拒绝duplicate key，不能依赖普通`BTreeMap`的
last-write-wins。

### 2. Strict shape与cross-field validation

对input与canonical artifact都验证：

- gateway entry identity必须由其canonical protocol surface经F351唯一identity owner重算一致；
- adapter plan kind必须与HTTP protocol surface的adapter kind一致；
- adapter args复用F351 validator；`http.context`要求同entry存在pre；
- protocol surface中的external sources必须与adapter args中真正跨external boundary的sources
  规范化后精确一致；
- handler/pre/guard callable id非空且三类字段不能互相被key、source selector或public path代替；
- 当前generation只接受HTTP entry；selector protocol必须与entry HTTP protocol一致；
- selector唯一；每个selector必须引用存在的key；每个gateway entry必须至少被一个selector引用；
  同一个key允许被多个selector引用；
- 零gateway entry与零ingress是合法一致状态；
- `operationBindings`允许为空。Deployment projection仍要求它与传入ServiceContract的operation集合
  精确一致，因此只有零operation contract可生成零binding deployment。

不得在本leaf校验handler/pre/guard的source可达性、generic声明、exact linked signature、external schema
推导或runtime codec；这些是C2/C3 owner。这里只校验已经typed的deployment input内部自洽。

### 3. Projection、identity与generation

- deployment projection逐值保留validated gateway entries，并把selector/key binding写入artifact；
- `DeploymentArtifactIdentity` preimage覆盖：
  - gateway entry key；
  - entry identity与protocol surface；
  - handler/pre/guard exact callable ids；
  - adapter plan kind、参数名与source；
  - selector到key的绑定。
- map插入顺序、gateway entry顺序和ingress输入顺序不得改变identity；上述任一语义值变化必须改变
  deployment identity。
- diagnostic text仍不进入identity。
- bump且只保留一个严格generation：
  - `ServiceDeploymentInput` schema `v1 -> v2`；
  - `ServiceDeployment` schema `v1 -> v2`；
  - deployment identity projection marker/prefix `v1 -> v2`。
- 新字段无default；旧wire、旧schema、旧identity prefix与旧
  `contractOperationId` ingress必须fail closed，不保留reader或fallback。

### 4. 旧consumer的checkpoint行为

- `compiler/driver/generated_deployment.rs`
  - 无external authoring时生成empty `gatewayEntries`与empty ingress；
  - HTTP在C2接线前继续明确fail closed；
  - WebSocket业务entry未冻结，任何现有WebSocket authoring也明确fail closed；
  - 删除旧WebSocket/HTTP `operation -> ContractOperationId` ingress parser/resolver，不能把旧路径藏在
    未调用helper里。
- deployment assembly或其它直接consumer若尚不能生成新的RuntimeAssembly linked gateway entry，
  必须对nonempty deployment gateway ingress给出明确的not-yet-linked错误；不得重新解释为service
  operation、静默丢弃或实现C3 runtime行为。
- repo内受strict必填字段影响的构造器、wire fixtures和golden同批更新。跨owner production consumer只可
  机械增加empty字段或显式fail closed，不得在本leaf实现runtime/Router/test-runner gateway执行。

开始修改前可以派一个有界只读子Agent，回答“`DeploymentIngressBinding`有哪些直接production consumer，
哪些必须在本checkpoint fail closed”这一具体问题。遵守workspace工作流：该子Agent不得再派下一层，完成
consumer清单后立即结束。

## 写入范围

主要owner：

- `artifact-model/src/deployment.rs`及必要的新leaf module、exports、schema constants；
- `artifact-identity/src/deployment/**`、identity constants与直接tests；
- `deployment/src/projection/**`、storage/assembly的直接fail-closed seam与fixtures；
- `compiler/driver/generated_deployment.rs`的旧ingress reader删除及直接tests；
- 因strict DTO字段/generation导致的repo内机械constructor/wire/golden更新。

禁止修改：

- F351 gateway identity算法或external schema语义；
- `ServiceManifestAuthoring.http` shape；
- compiler handler/pre/guard source resolution、generic availability、schema/codec derivation；
- runtime/linker/Host/Router的新gateway执行、transport wire或test-runner dispatch；
- WebSocket connect/receive/message DTO或业务消息约定；
- stable/live配置、lockfile、三仓库service源码。

若必须新增公共codec plan语义、改变F351 identity surface、或RuntimeAssembly无法通过显式fail-closed seam保持
编译，先报告主Agent，不自行吞并C2/C3。

## 验证

先列出并确认非零selector，再运行聚焦验证：

```bash
cargo test -p skiff-artifact-model deployment -- --list
cargo test -p skiff-artifact-identity deployment -- --list
cargo test -p skiff-deployment -- --list
cargo test -p skiff-artifact-model deployment
cargo test -p skiff-artifact-identity deployment
cargo test -p skiff-deployment
cargo test -p skiff-compiler --lib generated_service_deployment
cargo check -p skiff-artifact-model -p skiff-artifact-identity -p skiff-deployment -p skiff-compiler
git diff --check
```

必须有正/负证据覆盖：

- typed与raw HTTP entry；
- one entry/one selector、one entry/multiple selectors、zero/zero；
- missing/orphan key、duplicate selector、duplicate raw map key；
- identity/surface mismatch、adapter kind/source mismatch、context-without-pre；
- handler/pre/guard、adapter param/source、selector/key各自mutation改变deployment identity；
- map/list reorder identity稳定；
- zero-operation contract/deployment；
- stale schema/prefix、missing `gatewayEntries`、旧`contractOperationId` ingress拒绝；
- generated deployment HTTP/WebSocket旧reader均fail closed且production反搜无旧operation resolver。

不运行workspace/root、stable/live，不push。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f355-deployment-gateway`
- branch：`codex/p5-f355-deployment-gateway`
- 从包含本task的integration checkpoint创建；
- 先提交production/tests，再提交result；
- result记录exact base/commit/tree、验证矩阵、直接consumer fail-closed状态与C2/C3残余；
- worktree保持clean，不merge/rebase integration。
