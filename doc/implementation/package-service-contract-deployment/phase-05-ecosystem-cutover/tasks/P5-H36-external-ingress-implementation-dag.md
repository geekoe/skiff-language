# P5-H36 External ingress implementation DAG

状态：Implementation parent ready；C0 shared model/identity leaf running。`service.yml` named-route
authoring已由用户冻结。

## 直接父节点

- 权威设计冻结与审计入口：`P5-H35-external-ingress-surface-separation.md`
- compiler/artifact审计：`P5-F347-external-ingress-compiler-artifact-audit-result.md`
- runtime/Router审计：`P5-F348-external-ingress-runtime-router-audit-result.md`
- public generic审计：`P5-F349-public-generic-boundary-availability-audit-result.md`
- 三仓库迁移审计：`P5-F350-external-ingress-ecosystem-migration-audit-result.md`

以上父节点沿引用链连接唯一最高权威设计：
`../../../../architecture/package-service-contract-deployment.md`。Gateway/runtime职责细节由
`../../../../architecture/gateway-runtime-adapter-boundary.md`补充；两者冲突时以前者为准。

## Exact evidence checkpoint

四份审计合流后的integration evidence：

- commit：`ce4e6adbddf470a104c4ec41bd8ccd030347cead`
- tree：`b5293dd111ce0f2207a827052ba2c4448ef9eeee`
- branch：`codex/package-service-phase-05`

该checkpoint只证明当前事实与迁移范围，不是production候选。后续leaf必须从包含本parent及其直接任务的
更新checkpoint分支，且在result中记录自己的exact base/commit/tree。

## 不得再解释的冻结语义

1. `api.yml`始终是Package公开面owner；只有显式`serviceCall: true`的function/public-instance root
   进入service-to-service projection。没有marker的ServiceContract可以有零个operation。
2. 一个显式marker若投影为`BoundaryCallableProjection::Unavailable`，compiler必须一次报告该root的全部
   结构化原因；不得静默省略。未标记的generic或其它boundary-unavailable callable仍是合法Package API。
3. HTTP/WebSocket入口由`service.yml`拥有，直接解析当前Package中的精确handler/pre/guard callable；
   handler不要求public，不生成`ContractOperationId`，不进入ServiceContract、
   `PackageSchemaRequirements`或service dependency module。
4. External schema只为真实跨external boundary的source/sink形成entry-local结构：
   typed HTTP body/response、typed WebSocket message body及固定平台wire shape。`pre` context、guard值、
   WebSocket connection context和其它runtime内部值不进入external schema。
5. 私有named type可以贡献external structural shape，但它的source/public path、nominal identity、
   `PackageSchemaTypeId`或display name不得泄漏，也不得因ingress被补进`api.yml`、PackageLocalAbi、
   PackageSchema或ServiceContract。
6. 第一版拒绝generic function declaration作为handler/pre/guard；concrete signature可包含fully
   instantiated generic platform types。不得按`std.websocket`名称建立generic特例。
7. `GatewayEntryKey`是service-owner-local稳定键，不是内容identity。Selector只绑定key，同一个entry可被
   多个selector复用；第一版authoring不提供该复用语法。
8. `GatewayEntryIdentity`只标识external protocol surface。其canonical preimage包含entry/protocol kind、
   unary/stream mode、外部request/response/message shape、影响wire的标准source选择、
   WebSocket context expectation、公开external error projection及其它wire兼容性metadata。
9. `GatewayEntryIdentity`明确不包含selector、source selector、handler/pre/guard
   `PackageCallableId`、PackageArtifact/build、deployment policy、内部nominal identity、目标参数名、
   内部context type/codec identity或完整adapter execution plan。只换实现而wire不变时identity保持；
   deployment revision必须变化。
10. Runtime payload codec只从exact linked callable signature形成；Router只转发opaque bytes和平台
    metadata，不解析业务type layout。External schema只供协议、文档、diagnostics与identity projection，
    不是runtime codec source。
11. External ingress使用普通gateway request lane与`caller.kind=gateway`，但不伪造service caller。
    第一版service依赖仍严格经过service boundary，即使物理同进程。
12. 不保留旧`operation`/`ContractOperationId` ingress reader、dual path、fallback或兼容generation。
    Skiff尚未发布，旧artifact/fixture/corpus必须同批更新或明确fail closed。

## 目标对象关系

```text
api.yml serviceCall roots
  -> ServiceContract operations
  -> ContractOperationId

service.yml gateway entries
  -> GatewayEntryKey
  -> compiler-derived GatewayEntryProtocolSurface
  -> GatewayEntryIdentity
  -> deployment-owned exact callable / adapter execution plan

external selector
  -> GatewayEntryKey
  -> current activation + exact deployment revision
```

三条identity不能互相代替。`ServiceProtocolIdentity`不读取gateway entries；
`GatewayEntryIdentity`不读取implementation；`DeploymentArtifactIdentity`同时覆盖exact implementation、
gateway entry bindings与selector bindings。

## 串并行DAG

### C0：唯一共享checkpoint

- F351 `Gateway artifact model / identity`
  - 强类型`GatewayEntryKey`、`GatewayEntryIdentity`；
  - shared closed adapter/source vocabulary；
  - normalized external protocol surface与entry-local external schema DTO；
  - Rust canonical identity owner、validation、golden与mutation matrix。

任何compiler、deployment、runtime或Router consumer不得先于F351自行新增另一份gateway
surface/identity模型。

### C1：C0之后可并行

1. `api.yml serviceCall + generic availability`
   - parser/projector只选择显式marker；
   - generic public Local ABI/link保留，PackageSchema只投影eligible closure；
   - marked unavailable聚合结构化错误，unmarked unavailable不阻断Package。
2. `strict service.yml authoring`
   - 冻结用户YAML shape；
   - `http.routes`/`websocket.routes`是named mapping；mapping key就是`GatewayEntryKey`；
   - 每个named route把唯一selector与entry definition写在一起，compiler内部再拆成两个artifact事实；
   - 直接引用当前Package source callable；
   - 旧`operation`、旧`handlerArgs`、旧WebSocket `bind`和unknown fields全部fail closed。
3. `deployment gateway model`
   - `gatewayEntries: GatewayEntryKey -> resolved entry`；
   - `ingress: IngressSelector -> GatewayEntryKey`；
   - exact handler/pre/guard `PackageCallableId`与execution plan由deployment拥有；
   - bump deployment input/artifact identity generation，无旧reader。

三项只要F351通过即可准备独立leaf。

### C2：compiler projection convergence

同时依赖C1三项：

- 解析non-public top-level callable并校验exact link/signature；
- generic function handler/pre/guard fail closed；
- 从signature、adapter kind与source形成execution plan、entry-local external schema与
  `GatewayEntryIdentity`；
- ServiceContract只读取`serviceCall` roots；
- PackageArtifact不需要为private ingress type伪造public/schema record；
- 更新compiler receipt、artifact generation与严格negative probes。

### C3：C2之后可并行consumer

1. runtime loader/linker/Host：形成linked gateway entry，按key/identity/admitted plan执行普通gateway
   request lane；关闭HTTP stream Host断裂。
2. transport/Router：selector只映射gateway key/identity；删除Rust/TypeScript重复WebSocket synthetic
   identity；HTTP/WS只转发opaque payload和平台metadata。
3. WebSocket codec/generation：connect时pin exact entry/generation，receive沿旧generation entry；
   保留receipt、release ack、disconnect cleanup与drain gate。
4. test-runner：删除两个合成`IngressSelector -> ContractOperationId` owner，测试服务走同一gateway entry
   模型；不在runner复制identity或codec。

F346 fixed service error carrier、external redaction与restricted telemetry可以复用；若上述leaf修改F346声明的
production surface，最终验收必须按影响范围重跑。

### C4：共享链完成后的迁移

按F350清单拆为不重叠owner：

- Skiff compiler/runtime-live/legacy fixtures与canonical package/service roots；
- Internals Account与Codex Relay；
- Internals AIHub与Agine，包括WebSocket connect/receive；
- skiff-packages无ingress service只做新generation registry/release revalidation；
- test-runner与F269生成的test-service roots由其各自owner刷新，不得重新引入旧co-located test。

目标operation数量必须至少复核：

- Account `21 -> 0`
- Codex Relay `17 -> 2`
- AIHub `8 -> 6`
- Agine `2 -> 0`
- Registry保持`20`

### C5：合流、F269刷新与独立验收

1. 合流所有production consumer和三仓库迁移。
2. 同一F269 owner在新checkpoint上reconcile现有WIP，刷新receipt/identity并重跑canonical
   test-service workflow；不得由其它leaf改写其worktree。
3. 做跨对象convergence：identity mutation matrix、strict generation、ServiceProtocol invariance、
   Router/Host exact admission、HTTP raw/typed/stream、WebSocket generation与fixed error evidence。
4. 新独立agent做高风险只读验收；PASS后才能进入Phase 05总验收、main merge和worktree/临时分支清理。

## `service.yml` named-route authoring checkpoint

用户已经冻结合并写法，不维护独立`entries`与`routes`两张表：

```yaml
http:
  routes:
    createUser:
      method: POST
      path: /users
      kind: typedJson
      handler: users.create
      adapterArgs:
        - param: body
          source: { kind: http.body }

websocket:
  routes:
    chat:
      path: /ws
      connect:
        handler: chat.connect
        adapterArgs:
          - param: request
            source: { kind: websocket.connectRequest }
      receive:
        handler: chat.receive
        adapterArgs:
          - param: event
            source: { kind: websocket.receiveEvent }
```

`createUser`和`chat`就是owner-local稳定key，不再重复写`id`。Compiler从每个value同时投影一个selector
binding和一个resolved entry。第一版每个named route只有一个selector；多个route即使指向同一个handler也
仍是独立entry。Artifact层仍保持selector/key/entry分离，且identity preimage绝不能读取selector。

## 验证与运行边界

- 每个leaf先枚举非零selector，再运行其最小production/negative probes。
- 不用root/workspace test掩盖局部空跑；跨语言identity必须有唯一Rust owner及frozen golden/corpus。
- 开发leaf不得运行stable/live、修改stable instance或push。需要live/chat smoke的最终leaf必须另有明确
  授权与可执行task。
- 所有已合流worktree在验收通过后按workspace约定合回integration并删除；integration最终合回各仓库
  `main`，不push。
