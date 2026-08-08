# Router Rust Migration C-model-spawn：spawn wire 与 parent 模型冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-model-spawn / M-spawn /
`H-spawn-parent-cut` 消费）

> M-spawn-repair（2026-08-03）：按权威设计 §6.1（阻断迁移的 bug fix 先更新
> canonical contract/corpus，再修改所有消费者）修复冻结契约中的方向自相矛盾：
> `spawn.submit.request` 的真实 wire 方向是 Runtime→Router（Runtime driver
> 出站、TS Router inbound 处理），`spawn.submit.response/error` 是
> Router→Runtime；spawn family 为 mixed-direction，family 级 `Either` +
> 帧级 direction 表（见 §3.0、§6.1）。同时补齐 `SpawnSubmitAcceptance`
> 数据面（§7.2）：acceptance 必须携带重建出站 `spawn.submit.request`
> 所需的原始 wire header/payload（service/activation identity、
> actorMethod 元数据、args bytes）。`frameHex` golden bytes 不变。

## 引用链

- 权威设计：`doc/implementation/router-rust-migration/execution/router-rust-migration-plan.md`
  - §5.3（C-model-spawn → W-model-spawn → M-spawn → `H-spawn-parent-cut`；
    `callerKind = request | actorInvocation` 决策；删除靠字符串前缀猜测的
    fallback）；
  - §5.4（`C-spawn + M-spawn + H-spawn-parent-cut`：`FunctionSpawnParentResolver`
    + `ActorSpawnParentResolver` + stateless `SpawnSubmitRouter`；
    M-spawn/C-spawn 使用不可碰撞 typed parent kind/correlation namespace；
    必须测试 collision、parent terminal 和 replacement 竞态）；
  - §5.5（`SpawnSubmitRouter` 作为 stable sink bundle 的一员，sink 不拥有
    pending）；
  - §3.4（authority snapshot 的 identity/fence 类型）、§7（E-actor-rust：
    function spawn 与 actor-method spawn parent authority 明确，accepted
    spawn 与 parent 生命周期分离）。
- 父批次：`doc/implementation/router-rust-migration/execution/router-rust-migration-batch-4.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration/execution/router-rust-migration-contracts-actor-leaf.md`。
- 同链契约：`router-rust-migration-c-model-actor-contract.md`、
  `router-rust-migration-c-actor-contract.md`、
  `router-rust-migration-c-spawn-contract.md`。

冲突时以权威设计为准；本文件只冻结契约与 corpus，不写 production。

## 1. 冻结范围

冻结 spawn 族 wire 模型的目标形态：显式 closed enum `callerKind`、typed
parent namespace（request / actorInvocation）、`FunctionSpawnParentResolver`
/ `ActorSpawnParentResolver` / stateless `SpawnSubmitRouter` 的 model 边界，
以及 `H-spawn-parent-cut` 前置（旧 shape 删除、无兼容 reader）。不定义
`RequestDispatcher` / `ActorInvocationRelay` 内部 pending 实现
（C-actor/C-dispatch），不写 skiff-router production。

## 2. callerKind 决策（frozen）

当前 wire 只有 `targetKind + callerRequestId` 而没有 parent domain
（`SpawnSubmitRequestFrameHeader`）；`callerRequestId` 是裸字符串，Router
现在先在 request pending 查找、再查 actor invocation parent，两路都存在即
ambiguous——这就是设计要求删除的“靠字符串猜测”fallback。

冻结决策：

- spawn wire 新增 **required closed enum** `callerKind`：

  ```text
  callerKind = "request" | "actorInvocation"
  ```

- parent namespace 不可碰撞：parent correlation key = typed pair
  `(callerKind, callerRequestId)`；`callerKind` 决定唯一解析器：
  `request` -> `FunctionSpawnParentResolver`（RequestDispatcher 的 fenced
  authority snapshot）；`actorInvocation` -> `ActorSpawnParentResolver`
  （ActorInvocationRelay 的 fenced authority snapshot）。
- 禁止：用字符串前缀/内容猜测 parent domain；允许同一字符串
  `callerRequestId` 同时存在于 request 与 actorInvocation namespace
  （不碰撞，各自解析）；缺少 `callerKind` 的旧 shape 一律拒绝。
- 不改变 `targetKind`（`function | actorMethod`）语义：`targetKind` 是
  spawn 目标分类，`callerKind` 是 parent 来源分类，两者正交。
- 命名隔离：spawn 的 `callerKind` 字段只存在于 `spawn.submit.request`
  帧面，closed enum 精确为 `request | actorInvocation`。现有
  request-dispatch 面已有一个同名 `callerKind`（`gateway | service`，
  见 cross-system fixtures / runtime-registry-dispatch 测试），属于不同帧
  与不同语义，本契约不改动它；两个面之间不存在猜测或换算。
- frame schema generation 升级按现有规则进行（
  `H-spawn-parent-cut` 依赖 M-spawn），本契约冻结目标 wire，不提前实现。

## 3. 目标 wire（frozen shape）

### 3.0 方向（M-spawn-repair 修复后的 canonical 事实）

spawn family 是 mixed-direction 族：family 级 registry 标注 `Either`，
帧级 direction 表如下（demux/consumer 必须按帧收窄，禁止把 family 级
`Either` 当作任意方向都合法）：

| 帧 | 方向 |
| --- | --- |
| `spawn.submit.request` | RuntimeToRouter（Runtime driver 出站） |
| `spawn.submit.response` | RouterToRuntime（Router 出站） |
| `spawn.submit.error` | RouterToRuntime（Router 出站） |

- `spawn.submit.request` 是唯一 inbound 帧：Router 侧在
  `validateRuntimeToRouterFrameHeader` 与 demux 的 RuntimeToRouter 面消费。
- `spawn.submit.response` / `spawn.submit.error` 是唯一 outbound 帧：
  Runtime 侧按 inbound 消费（`rpcId` correlation）。
- 不存在额外的 Router→Runtime forwarding/accept 帧：accept/reject 就是
  `spawn.submit.response` / `spawn.submit.error`。
- correlation 形态：request 携带 `rpcId`，response/error 回显同一 `rpcId`；
  response 额外携带 Router 生成的 `spawnId` + `requestId`（accepted spawn
  的后续执行 correlation 唯一键）。
- 任何方向违例（Router 收到 response/error、Runtime 收到 request）按
  protocol violation fail closed，无兼容 reader。

### 3.1 SpawnSubmitRequestFrameHeader（target）

```text
{
  schemaVersion: "skiff-runtime-frame-v3",
  type: "spawn.submit.request",
  rpcId: <token>,
  runtimeId: <token>,
  callerKind: "request" | "actorInvocation",     // 新增，required
  callerRequestId: <token>,                       // required，typed namespace 内唯一
  targetKind: "function" | "actorMethod",
  serviceId: <token>,
  serviceVersion: <token>,
  serviceProtocolIdentity: <token>,
  target: <token>,
  spawnId?: <token>,
  buildId?: <token>,
  activationIdentity: { assemblyIdentity, generation, runtimeReplicaId, deploymentRevision },
  traceId?: <non-empty string>,
  callerTarget?: <token>,
  maxQueueWaitMs?: <number>,
  actorMethod?: SpawnActorMethodTargetFrameMetadata,
}
```

- `actorMethod` 在 `targetKind == "actorMethod"` 时必须存在；
  `targetKind == "function"` 时不得携带（corpus 正负例断言）。
- payload presence：Spawn family = Required（M0 已冻结；payload 为不可变
  opaque function/actor arguments bytes，codec 不解释）。
- serde：camelCase + `deny_unknown_fields`；identity 字段沿用
  `skiff-actor-abi-v1:sha256` 等 framed 校验。

### 3.2 SpawnSubmitResponseFrameHeader

```text
{ schemaVersion, type: "spawn.submit.response", rpcId, spawnId, requestId, status: "submitted" }
```

- `requestId` 是 Router 为 accepted spawn 生成的独立 invocation/request
  id；accepted spawn 与 parent 生命周期分离（parent terminal 不影响
  accepted spawn 的执行），`requestId` 是后续 correlation 的唯一键。

### 3.3 SpawnSubmitErrorFrameHeader

```text
{ schemaVersion, type: "spawn.submit.error", rpcId, error: RuntimeErrorFramePayload }
```

- 错误码（frozen closed set）：`ParentNotFound`、`ParentTerminal`、
  `ParentReplaced`、`ParentConnectionMismatch`、`CallerKindRejected`
  （缺/非法 `callerKind`，旧 shape）、`TargetKindMismatch`、
  `AuthorityMismatch`、`Saturated`、`UnknownTarget`。

## 4. Resolver / Router model（stateless）

### 4.1 FunctionSpawnParentResolver

- 输入：`(callerKind="request", callerRequestId)` + 捕获的
  `RequestDispatcher` fenced authority snapshot
  （`RuntimeSpawnParentAuthority`：runtimeId、buildId、serviceProtocolIdentity、
  assemblyIdentity、assemblyGeneration、deployment exact tuple）。
- 输出：`SpawnParentResolution { kind: request, parent_request_id,
  authority, origin_runtime_connection }` 或拒绝。
- 校验：parent request 存在、在同一 runtime connection、authority 未
  过期（epoch/assembly exact）、testCaseCapability 配对时 authority
  一致；任何不满足 -> fail closed。
- resolver 不拥有 pending：只按传入 snapshot 查询，不注册/不删除 parent。

### 4.2 ActorSpawnParentResolver

- 输入：`(callerKind="actorInvocation", callerRequestId)` + 捕获的
  `ActorInvocationRelay` fenced authority snapshot（同一 authority 类型 +
  invocation fence：origin runtime/connection、testCaseCapability）。
- 输出：`SpawnParentResolution { kind: actorInvocation,
  parent_invocation_id, authority, origin_runtime_connection }` 或拒绝。
- 校验：invocation pending 存在、caller 与 origin connection 精确一致、
  authority exact（含 testCaseCapability）、owner 仍在位；不满足
  -> `ParentTerminal` / `ParentReplaced` / `ParentConnectionMismatch`。
- resolver 不拥有 pending。

### 4.3 SpawnSubmitRouter（stateless sink）

- 单一入口 `submit(frame) -> Result<SpawnSubmitAcceptance, SpawnSubmitError>`；
- 按 `frame.callerKind` 精确选择 resolver：`request` 只走
  FunctionSpawnParentResolver，`actorInvocation` 只走
  ActorSpawnParentResolver；**不存在 fallback、不存在跨 namespace 查找**；
- 随后按 `targetKind` 分类目标（function / actorMethod），生成
  `requestId` 并返回 `spawn.submit.response`；
- sink 不拥有 pending：accepted spawn 的后续执行/取消由
  RequestDispatcher / ActorInvocationRelay 以 `requestId` 关联，
  `SpawnSubmitRouter` 不保存 parent-child 映射、不做 terminal 补偿。

## 5. H-spawn-parent-cut 前置（frozen）

- `H-spawn-parent-cut` 依赖本 pack + M-spawn：current Runtime 与 TS Router
  必须先通过同一 shared corpus，再硬切消费目标 wire。
- cut 内容：删除旧 shape（无 `callerKind` 的 `spawn.submit.request`），
  **无兼容 reader**——不允许“缺 callerKind 时猜测/默认 request”的 reader；
  旧帧在任意阶段被拒绝（`CallerKindRejected`）。
- 本 pack 冻结目标 wire 与 cut 规则；不实现 cut（M-spawn / W-spawn
  职责），不提前写 skiff-router production。

## 6. Byte-exact corpus 规格

位置：`runtime/transport/testdata/spawn-wire/`。

### 6.1 frames.json（帧目录）

```json
{
  "schemaVersion": 1,
  "corpus": "spawn-wire-v1",
  "frames": {
    "<frame-name>": {
      "direction": "RuntimeToRouter | RouterToRuntime",
      "frameType": "spawn.submit.request | spawn.submit.response | spawn.submit.error",
      "decodeAs": "SpawnSubmitRequest | SpawnSubmitResponse | SpawnSubmitError",
      "payloadPresence": "required | empty",
      "payloadBase64": "<payload>",
      "frameHex": "<完整二进制帧 hex>",
      "legacyCut": false,
      "header": { "...": "typed header 语义 JSON" }
    }
  }
}
```

方向按 §3.0 帧级表冻结：三个 `spawn.submit.request` 帧为
`RuntimeToRouter`，`spawn.submit.response` / `spawn.submit.error` 为
`RouterToRuntime`。`frameHex` 是 byte-exact 事实，方向标注修复不改变
golden bytes。

必选帧：

- `submit.request.function`（`callerKind=request`，function target）；
- `submit.request.actorMethod`（`callerKind=actorInvocation`，actorMethod
  target + `actorMethod` metadata）；
- `submit.response.submitted`；
- `submit.error.parentNotFound`；
- `submit.request.legacy-no-caller-kind`（旧 shape；`legacyCut: true`，
  目标 mirror decode 必须拒绝，且不存在任何兼容 reader）。

`frameHex` 是 byte-exact 事实：`encode(decode(hex)) == hex`（decode 走
测试内 target mirror struct + `encode_binary_frame`；新字段尚未在
production codec 实现，因此 frameHex 由 mirror 生成，cut 后由真实 codec
接管同一 corpus）。

### 6.2 scenarios/（parent 语义序列）

```json
{
  "schemaVersion": 1,
  "scenario": "<name>",
  "parents": { "request": [ "<requestId>", ... ], "actorInvocation": [ "<invocationId>", ... ] },
  "events": [
    { "op": "submit", "callerKind": "request", "callerRequestId": "<id>", "targetKind": "function" },
    { "op": "parentTerminal", "callerKind": "request", "callerRequestId": "<id>" },
    { "op": "replace", "callerKind": "actorInvocation", "callerRequestId": "<id>" }
  ],
  "expect": { "accepted": [...], "rejected": [...], "errors": { "<callerKind>:<id>": "<error>" } }
}
```

必选场景（测试同文件断言存在）：

- `resolve-function-parent-exact`
- `resolve-actor-invocation-parent-exact`
- `same-request-id-both-namespaces-no-collision`
- `missing-caller-kind-legacy-cut-rejected`
- `parent-terminal-before-submit-rejected`
- `parent-replaced-before-submit-rejected`
- `parent-connection-mismatch-rejected`
- `authority-mismatch-rejected`
- `accepted-spawn-outlives-parent-terminal`
- `target-kind-mismatch-rejected`

## 7. §5.4 contract pack 必填项

### 7.1 owner / invariant

- Owner：`FunctionSpawnParentResolver`（request parent 解析）、
  `ActorSpawnParentResolver`（actor invocation parent 解析）、
  `SpawnSubmitRouter`（exact parent kind 选择 + 目标分类 + acceptance）；
  accepted spawn 的执行归 RequestDispatcher / ActorInvocationRelay。
- Invariant：parent correlation 严格 typed `(callerKind,
  callerRequestId)`；同一字符串跨 namespace 不碰撞；resolver 不修改
  parent pending；router 不拥有 pending；旧 shape 永远拒绝。

### 7.2 typed inputs / outputs

- Inputs：`SpawnSubmitRequestFrameHeader`（target shape）、
  `RuntimeSpawnParentAuthority` snapshot、parent pending 查询 port。
- Outputs：`SpawnParentResolution`、`SpawnSubmitAcceptance`、
  `spawn.submit.response` / `spawn.submit.error`。
- `SpawnSubmitAcceptance`（M-spawn-repair 补齐数据面）：

  ```text
  SpawnSubmitAcceptance {
    request: SpawnSubmitRequestFrame {
      header: SpawnSubmitRequestFrameHeaderV2,  // 原始 wire header
      payload: Vec<u8>,                          // 不可变 opaque args bytes
    },
    spawn_id: String,
    request_id: String,
  }
  ```

  acceptance 边界必须携带重建出站 `spawn.submit.request` 所需的原始 wire
  header/payload（service/activation identity、actorMethod 元数据、args
  bytes）或等价 typed 投影，供真实执行 sink 使用（E-actor-rust 前置）；
  `request_id` 是 Router 生成的后续执行 correlation 唯一键。执行 sink
  不得重新 parse 原始 bytes，也不得丢失任何 wire 字段。

### 7.3 capacity

- spawn submit 并发上限：`runtime.maxConcurrency`（共享 dispatcher 预算）；
  超过 -> `Saturated` 立即拒绝；
- 单次 submit 的 resolver 查询有 deadline（默认 5s，超时
  `ParentTerminal`）；
- authority snapshot 不复制 payload，只复制 typed refs。

### 7.4 queue full

- spawn family writer queue 沿用 C-session per-session 预算（outbound
  256 帧 / 4 MiB；enqueue non-blocking）；满 -> abort exact runtime
  connection，submit 按 disconnect terminal 收敛；
- resolver 查询不排队无限等待：超时/饱和 fail closed。

### 7.5 timeout / disconnect / replacement / shutdown terminal

- timeout：resolver 查询超时 -> `ParentTerminal`；submit 已 accepted 后
  parent terminal 不影响 accepted spawn（生命周期分离）；
- disconnect：parent origin runtime 断开 -> 未 accepted 的 submit 拒绝
  （`ParentConnectionMismatch` / `ParentTerminal`）；已 accepted 的 spawn
  由执行 owner 按 disconnect 收敛；
- replacement：parent connection/replica 被 replacement 替换 -> 旧
  authority 的 submit 拒绝（`ParentReplaced`）；new parent 不得继承旧
  submit；
- shutdown：全部 submit pending / resolver 查询归零，
  C-process-lifecycle 覆盖。

### 7.6 health fields

- 按 `callerKind` 的 submit 计数（request / actorInvocation）、accepted /
  rejected 计数、错误码分布（closed set）、resolver 查询占用/超时、
  legacy-cut 拒绝计数、writer queue 占用。

### 7.7 fake seam

- `FakeRequestParentStore` / `FakeActorInvocationParentStore`（内存 pending
  + 可注入 terminal/replacement/connection 变化）、`FakeClock`。
  corpus 测试消费 fixtures + 参考 router；W-model-spawn 用同一 fixtures
  通过真实 codec。

### 7.8 real boundary probe（定义，M-spawn/W-spawn/E-actor-rust 执行）

- `router-live:spawn-parent-cut` probe：真实 Router + fake Runtime：同一
  `callerRequestId` 分别以 `callerKind=request` 与
  `callerKind=actorInvocation` 提交，断言互不碰撞；parent 断开/替换后
  submit 拒绝；旧 shape 帧在 cut 后被关闭且无 fallback。该 probe 成为
  `router-rust-actor-live` 的 required managed CI。
