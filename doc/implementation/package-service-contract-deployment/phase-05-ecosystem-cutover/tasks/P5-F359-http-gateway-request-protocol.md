# P5-F359 HTTP gateway request protocol

状态：Ready（C3 shared Rust/TypeScript wire checkpoint；依赖F358）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F348-external-ingress-runtime-router-audit-result.md`
- `P5-F358-runtime-assembly-http-gateway-linking-result.md`
- `P5-F267-inline-effect-runtime-cutover-result.md`

以上父节点沿引用链连接唯一权威设计。本任务只完成 RuntimeAssembly HTTP gateway request 的
shared Rust/TypeScript transport contract；不得重新设计 HTTP authoring、linked entry、
runtime codec、response framing、错误投影、timeout执行责任或WebSocket业务消息入口。

## Exact base

- integration commit：`d26b56e78871c86b783944c54971886defa71e98`
- integration tree：`40b18203fefa395cffd9dc47fe8b088a8147cecd`
- branch：`codex/package-service-phase-05`

当前checkpoint已经包含：

- RuntimeAssembly v2 `gatewayIngress`；
- selector到exact deployment/key/identity的binding；
- loader/linker提供handler/pre/guard exact target、signature、protocol surface与adapter plan；
- Host、Router与test-runner尚未迁移，仍是明确downstream consumer。

## DAG位置与解除条件

```text
F358 linked gateway entry
  -> F359 shared request wire
       -> Host/request/eval consumer
       -> Router snapshot/HTTP consumer
       -> test-runner gateway execution
```

本任务完成后仍只是实现检查点。它只证明两端严格编码/解码同一请求声明，不证明Router已经生成该
请求，也不证明Host已经admit或执行handler。

## 冻结后的请求边界

RuntimeAssembly gateway request只允许HTTP。Router发送的可信边界压缩为：

```text
assembly identity + generation
+ HTTP ingress selector
+ GatewayEntryIdentity claim
+ request-specific HTTP metadata / opaque body
+ request lifecycle metadata
```

handler、pre、guard、adapter plan、callable signature、activation、deployment、codec和diagnostic
target全部从Host已admit的F358 linked entry取得，不由Router注入。

## 必须完成

### 1. Canonical routing header

Rust与TypeScript的canonical routing必须严格一致：

```json
{
  "kind": "runtimeAssembly",
  "assemblyIdentity": "skiff-runtime-assembly-v2:sha256:<64 lowercase hex>",
  "assemblyGeneration": 7,
  "gatewayEntryIdentity": "skiff-gateway-entry-v1:sha256:<64 lowercase hex>",
  "ingress": {
    "protocol": "http",
    "host": "example.com",
    "method": "POST",
    "path": "/items"
  }
}
```

要求：

- `gatewayEntryIdentity`是required routing field，并使用artifact-model的strict
  `GatewayEntryIdentity`；不得保留旧top-level optional identity；
- 删除`contractOperationId`；
- 不传`gatewayEntryKey`、deployment ref、activation identity或ServiceContract事实；Host用selector
  命中linked entry并exact-match identity；
- ingress protocol只接受`http`，method必须是非空字符串，host非空，path为absolute；
- 旧WebSocket RuntimeAssembly routing、旧contract operation routing、unknown/missing/duplicate
  field全部fail closed；
- 不新增serde alias、default、dual read、fallback或legacy adapter。

### 2. Canonical request header

canonical RuntimeAssembly request继续使用既有binary frame与
`schemaVersion: skiff-runtime-frame-v1` / `type: request.start` framing；该version拥有通用frame
格式，不是旧RuntimeAssembly routing generation。

Header允许且只允许：

- required `schemaVersion`、`type`、`requestId`、`mode`；
- required `caller: { kind: "gateway" }`，删除可伪造的`caller.target`；
- required `routing`；
- optional `clientSession`；
- optional `deadline`；
- required `trace`；
- required `httpRequest`；
- optional/default-false `testEffectsEnabled`。

其中：

- `mode`仍只允许`unary | serverStream`，后续Host必须与linked protocol surface exact-match；
- `deadline`保留既有strict request-specific shape，本leaf不决定谁生成或执行timeout；
- `httpRequest`只携带真实method/url/path/query/headers等request metadata；
- binary payload继续是opaque request body，Router不解析业务type layout；
- `testEffectsEnabled`只授权compiler-generated inline effect setup运行，默认false；
- 删除`testEffectDoubles`。F267后effect内容已经编译进隐藏setup，不再从wire注入；
- 删除`activationIdentity`、top-level `gatewayEntryIdentity`、`businessIdentity`、
  `websocketEntryId`、`httpAdapter`、`websocketAdapter`；
- static handler/guard/pre/adapter args不得以其它名字重新进入wire。

General legacy manifest request DTO若仍服务尚未迁移的非RuntimeAssembly路径，可以留在其独立owner；
它不得被canonical RuntimeAssembly decoder接受，也不得作为本任务正例。不要为了本checkpoint删除整个
legacy manifest协议；最终删除由其既有owner/Phase 05收尾负责。

### 3. Rust唯一typed decoder

`skiff-runtime-transport`必须：

- 以artifact-model `AssemblyIdentity`和`GatewayEntryIdentity`作为typed routing字段；
- 删除RuntimeAssembly-specific WebSocket、adapter、activation和test-double decode路径；
- strict raw JSON继续拒绝duplicate/escaped duplicate、unknown、missing、wrong nullability、
  negative zero、unsafe generation与错误identity prefix；
- encode/decode对canonical header与opaque payload精确roundtrip；
- HTTP payload可以为空或非空，不因adapter kind自行解释；
- legacy普通`RequestStartFrameHeader` baseline与canonical RuntimeAssembly decoder保持互斥。

不要在transport中读取F358 linked entry或实现Host admission；transport只拥有wire。

### 4. TypeScript唯一validator/decoder

Router protocol owner必须与Rust逐字段同构：

- `runtimeAssemblyRequest.ts`及已拆分的strict/json/frame/metadata模块拥有canonical shape；
- `runtimeProtocol.ts`只保留schema注册、thin delegation和通用legacy路径，不再复制一套
  RuntimeAssembly field/semantic validator；
- TypeScript使用唯一`runtimeAssemblyIdentity` lexical owner并升级为v2；
- gateway identity validator只接受`skiff-gateway-entry-v1:sha256`；
- raw JSON decoder必须像Rust一样拒绝duplicate与escaped duplicate keys；
- Router-to-runtime direction接受canonical routing，runtime-to-router direction继续拒绝；
- protocol层不得读取snapshot、deployment、ServiceContract或选择handler。

`runtimeProtocol.ts`已经是超长文件。本任务不得继续往其中增加完整validation逻辑；若必须调整它，只做
删除重复、字段表或thin call wiring。直接触碰的重复RuntimeAssembly validation应收敛到现有拆分模块。

### 5. RuntimeAssembly identity v2 lexical convergence

F358已把Rust canonical prefix升级为v2。本任务作为第一条跨语言wire checkpoint，必须同步：

- TypeScript assembly activation lexical validator与错误文本；
- request/control/activation状态中直接使用该validator的strict协议测试；
- cross-system fixtures中属于当前RuntimeAssembly identity的合法v1 prefix改为v2；
- canonical empty assembly oracle使用F358结果中的真实v2 identity：
  `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f`。

允许机械更新直接受影响的：

- activation request/state/control/raw cases；
- runtime activation frame；
- ecosystem store cases；
- WebSocket generation lifecycle tuple中的assembly identity prefix；
-对应artifact-model/runtime-transport/Router protocol tests。

这只是同一identity generation的机械consumer迁移。不得修改Router filesystem RuntimeAssembly
identity recomputation或snapshot loading；F358已经明确它们由后续Router consumer删除/迁移。不得借
此任务重设计WebSocket entry identity、generation pin或response协议。

### 6. Shared cross-system corpus

重写`runtime-request-wire.json`及checkpoint field declaration，使它们只描述上述canonical HTTP
request。Corpus必须由Rust与TypeScript共同消费并至少包含：

- 非空的canonical正例：full metadata + nonempty payload、minimal/defaults、`unary`与
  `serverStream`；
- nonempty mutation matrix，逐字段覆盖required/optional/unknown/type/nullability、identity、
  selector、direction与payload边界；
- raw JSON duplicate/escaped duplicate、negative zero、unsafe integer与invalid UTF-8/JSON；
- optional/default等价对；
- 独立legacy ordinary request baseline，且canonical decoder必须拒绝。

不要在任务中冻结某个方便的mutation数量；verifier/tests必须读取实际非零集合并检查名字唯一、每个case
真实执行。旧WebSocket正例和`ContractOperationId`正例全部删除；它们应改为明确负例或反向搜索证据。

### 7. 不受待决语义影响

本任务不决定：

- `typedJson`是否支持server stream；
- stream framing、content type、backpressure、terminal/error表示；
- Router与Host如何共同执行deadline；
- HTTP adapter的runtime codec；
- WebSocket application message routing。

Wire只保留现有`mode`与`deadline`字段；后续consumer依据用户冻结语义和linked entry执行。如果实现发现
wire本身必须先决定以上问题，立即报告`TASK_NOT_EXECUTABLE`，不得自行选择。

## 写入范围

主要owner：

- `runtime/transport/src/runtime_assembly_request*`及直接tests；
- Router `src/protocol/runtimeAssemblyRequest*`；
- Router `src/protocol/runtimeProtocol.ts`中canonical RuntimeAssembly的thin schema/delegation；
- Router `src/protocol/envelope.ts`中直接typed surface；
- `router/src/protocol/assemblyActivationLexical.ts`及直接protocol tests；
- `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`、
  `checkpoint.json`、`verify.mjs`；
- 第5节列明的identity v2直接fixture/tests；
- 必要的同crate exports。

允许删除只服务旧RuntimeAssembly operation/WebSocket/adapter/test-double wire的types、validators与
tests。直接触碰的metadata模块若删除大量职责，应按HTTP request/lifecycle职责保持可读结构，不能留下空壳
或第二套validator。

禁止修改：

- F351/F355/F358 artifact、deployment、RuntimeAssembly、linked entry模型或identity算法；
- compiler与HTTP authoring；
- runtime loader/linker；
- Host/request/eval/activation admission；
- Router snapshot、runtime registry、HTTP/WebSocket gateway或dispatch；
- test-runner；
- response/error/cancel/connection.send wire；
- stable/live配置、lockfile或三仓库service源码。

若TypeScript type-check因尚未迁移的Router builders失败，在result中列出精确consumer；不得越界修改
`assemblyHttpGateway.ts`、`assemblyWebSocketGateway.ts`、snapshot或Host来制造workspace绿色。当前
checkpoint允许下游consumer暂时编译断链。

## 完成标准与验证

先枚举并确认selector非零，再运行：

```bash
cargo test -p skiff-runtime-transport runtime_assembly_request -- --list
cargo test -p skiff-artifact-model assembly_activation -- --list
cargo test -p skiff-runtime-transport assembly_activation -- --list
cargo test -p skiff-runtime-transport websocket_generation_lifecycle -- --list

cargo test -p skiff-runtime-transport runtime_assembly_request
cargo test -p skiff-artifact-model assembly_activation
cargo test -p skiff-runtime-transport assembly_activation
cargo test -p skiff-runtime-transport websocket_generation_lifecycle
pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
cargo check -p skiff-runtime-transport
rustfmt --edition 2021 --check <changed-rust-files>
git diff --check
```

若某个filter为零，改用能证明对应owner的非零精确filter并在result记录实际枚举。Router完整
type-check若被未迁移builder阻断，只作为明确downstream证据，不是本leaf通过条件；protocol test与
cross-system verifier必须真实通过。

反向搜索至少证明canonical RuntimeAssembly Rust/TypeScript modules与shared request corpus中没有：

- `ContractOperationId` / `contractOperationId`；
- top-level `gatewayEntryIdentity`；
- `activationIdentity`；
- handler/guard/pre/adapter args；
- `testEffectDoubles`；
- WebSocket ingress/adapter/business fields；
- assembly identity v1合法正例。

General legacy protocol、明确拒绝旧字段的negative test或错误文案可以保留命中，但必须逐项分类，不能把
production canonical残留当作test-only。

不运行workspace/root、stable/live，不push。

## 风险、证据与验收

- 风险：高（Rust/TypeScript shared wire、strict raw decoder与identity generation consumer）。
- 开发Agent拥有上述聚焦Rust/TypeScript/cross-system证据。
- 合流后主Agent只运行一次便宜combined corpus probe，不重复两端完整聚焦套件。
- F359与Host/Router consumer合流后，由单一高风险验收owner检查最终wire/admission边界；本leaf不单独
  运行workspace/root gate。
- 任何wire field、identity lexical、raw decoder或corpus修改都会使本leaf证据失效。

## 有界探查与强制停止

开发Agent可为一个阻止实现的具体未知量派至多一个只读子Agent，例如确认
“现有cross-system corpus中哪些文件由assembly identity lexical直接消费”。必须限定路径、返回表格和
停止条件；该子Agent不得再派下一层。

从Agent启动到第一次实际代码修改默认不超过5分钟。若有界探查证明：

- canonical request仍必须携带未冻结的codec/stream/timeout/WebSocket语义；
- general runtime frame version必须整体升级并形成新的独立DAG owner；
- shared corpus无法在不修改Host/Router dispatch的情况下真实验证；
- 实际需要拆成互相依赖且不可在一个shared checkpoint闭合的多个公共wire owner；

则立即停止并返回：

- `TASK_SCOPE_EXPANDED`：被证伪前提、精确路径、实际owner、有效提交与最小拆分；
- `TASK_NOT_EXECUTABLE`：缺失决策及最小前置。

不得静默扩修、继续派Agent或把只读探查写成Completed。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f359-gateway-request-protocol`
- branch：`codex/p5-f359-gateway-request-protocol`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入同目录`P5-F359-http-gateway-request-protocol-result.md`；
- result记录exact base/production commit/tree、final field sets、Rust/TS strict decoder、
  identity v2 consumer、corpus case counts、nonzero selectors、反向搜索、下游编译断点与自验收矩阵；
- worktree保持clean，不merge/rebase integration，不push。
