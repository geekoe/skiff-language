# Runtime 错误分类与 Skiff 异常投影

Status: canonical architecture contract

Last updated: 2026-08-11

## 1. Scope

本文定义 Rust runtime failure 如何分类、记录诊断、在 request 内投影成 Skiff
异常，以及未被投影的 failure 如何进入 request、service、external ingress、control plane
和后台执行终态。

本文负责：

- 区分内部执行终态、Rust typed error、Skiff user exception 和已经固定的 service failure。
- 规定哪些错误信息属于程序语义，哪些只属于诊断。
- 规定 Rust error 获得 Skiff projection 的显式声明和 compiler/codegen owner。
- 规定 request phase、活跃 Skiff call site 和 operation policy 对实际投影的共同约束。
- 规定 deep source error、recoverable error、跨 service error 和 ingress error 的处理边界。

本文不负责：

- 为函数、Package ABI 或 ServiceContract 增加 checked exception / throw set。
- 把所有 Rust error 合并成一个 Skiff `RuntimeError` 类型。
- 定义业务预期失败；这类失败仍应优先使用业务返回 union。
- 规定 telemetry 消费、聚合或告警实现。

Skiff 尚未发布；实现应直接收敛到本文模型，不保留旧 projection table、旧字符串
identity 或旧 catch 行为的兼容层。

## 2. Position

Runtime error 由两个正交维度决定：

1. **错误是什么**：它是否有程序需要处理的专门语义类型，还是只需要形成诊断。
2. **错误发生在哪里**：当前是否存在仍可继续执行的 Skiff request frame / call site，以及该
   operation 是否允许把此错误交给业务代码处理。

某个 Rust failure class 能经 semantic adapter 生成 Skiff payload，不代表它每次出现都自动成为 Skiff
exception。反过来，
一个 failure 发生在 request 内，也不代表它必然可被 `catch<E>` 捕获。

最终规则是：

```text
Rust typed error
  -> source-operation semantic classification
  + generated Skiff projection DTO/declaration（可选）
  + current failure site
  + operation admission policy
  -> Skiff exception / request failure / external response / control failure /
     background terminal / internal stop
```

## 3. Runtime failure categories

### 3.1 Internal execution terminal

`Cancelled`、scope terminal、ancestor stop 等只用于 runtime 内部执行控制。它们不是 ordinary
error，不产生 public error payload，不获得 catch identity，也不能因为经过 host、request 或
transport 层就变成普通异常。

Deadline 与 instruction budget 不能共用一个 generic timeout identity。只有词法 `timeout(...)` owner
产生 `std.error.TimeoutError`；instruction limit 使用
`std.error.InstructionLimitExceededError`；HTTP primitive、Actor method invocation 与 Actor activation
分别使用自己的 public type。Request/root/inherited deadline以及用于停止未完成work item的cancellation carrier
仍是internal/request terminal，不能注入即将结束的frame或与可捕获错误混淆。

### 3.2 Rust typed error

当 Rust 代码需要根据 failure 的字段决定重试、fallback、资源释放、公开 payload 或其它行为时，
failure 必须使用专门的 enum variant / struct 保存这些字段。例如：

- provider unavailable 的 `target` / `reason`；
- protocol error 的 `target` / `message`；
- DB conflict 的 `retryable` 与逻辑 target；
- recoverable failure 的稳定 code；
- HTTP error 的公开 detail（若对应 Skiff 类型确实声明该字段）。

专门类型可以实现生成式 Skiff projection，也可以永远只在 Rust 内部处理。是否存在专门类型与
是否向 Skiff 暴露是两个独立事实。

### 3.3 Diagnostic-only Rust error

如果字段不参与 Rust 或 Skiff 程序决策，只用于开发者阅读，则不为它建立全局、穷举的
`RuntimeFaultData`。这类信息应进入：

- error `message`；
- Rust `source` error chain；
- runtime 逐层追加的 diagnostic frame；
- 受限 telemetry 所需的少量结构化 diagnostic attribute。

“业务代码不处理”不等于“telemetry 永远不需要结构化查询”。需要聚合的
`boundaryKind`、有限 reason 等可以作为受限 diagnostic attribute，但它们不是 public error
schema，也不要求进入所有 crate 共享的 Rust enum。

### 3.4 Request-local user exception

`RequestException` / `UserException` 保存：

- 实际 Skiff value 和 exact `CatchIdentity`，或无法在本地 materialize 的 opaque service cause；
- 当前 request 的 source site 和 exception stack；
- `traceId` / `errorId` correlation；
- 已经固定的 service error（若来自远端）。

它不是普通 Rust diagnostic struct，不能扁平化成 `code + message + stack` 后继续保持
`catch`、`rethrow`、opaque forwarding 和 exact nominal identity 语义。

### 3.5 Fixed service failure

`OpaqueServiceError` / `FixedServiceFailure` 保存已经严格验证并固定的
`ServiceErrorEnvelope` 及原始 canonical bytes。中间 service 未链接实际错误类型时必须原样转发，
不能把它降成 generic runtime diagnostic 后重新编码。

## 4. Diagnostic contract

### 4.1 Common diagnostic shape

所有非 internal-terminal failure 都必须能在负责记录它的边界形成以下逻辑诊断：

```text
RuntimeDiagnostic {
  code
  message
  diagnosticFrames
  correlation?
}
```

这是统一的diagnostic capability/view，不是要求所有Rust error共享同一struct、enum字段或继承层次。
Generated platform error唯一统一的public/wire外壳是`PlatformError` envelope；envelope内payload字段仍只来自
对应exact `.skiff` type IR。Diagnostic message、Rust source chain与diagnostic attributes都不因此成为public
schema。

- `code` 是有限、低基数的机器标签。Rust 内优先使用 enum / constant，只有在日志或 wire
  DTO 投影时转成 string。
- `message` 面向人阅读，可以包含不需要程序处理的上下文，但必须在进入 external/service
  response 或普通业务日志前完成脱敏和限长。
- `diagnosticFrames` 由 eval / host 在 failure 向上经过 source/call boundary 时追加；底层
  boundary/native error 创建时不要求知道 Skiff source stack。
- `correlation` 由 request/exception owner 分配；无 request 的 control/background failure
  可以没有 request correlation。

现有 `RuntimeErrorPayload { code, message, status, details }` 是 control/internal/legacy response
使用的扁平 DTO，不是 Skiff error type，也不是判断 catchability 的依据。`status` 属于具体外部协议
adapter；开放 `details: Json` 不能替代 typed error schema。

### 4.2 Three different stacks

实现必须区分：

1. Rust `Backtrace`：定位 runtime 实现 bug，只进入 operator 诊断，不稳定也不跨业务边界。
2. Runtime diagnostic frames：记录 source id、callable 和内部阶段，完整版本只进入受限 telemetry。
3. Skiff exception stack：属于 `Exception<E>` 的 request-local 语言语义；跨 service 后由 caller
   在自己的 call site 建立新栈。

三者不能合并成公开 error value 的一个 `stacktrace` 字段。公开 Skiff error payload 只保存该错误
类型声明的业务可见字段；stack 和 correlation 由 exception / service envelope 承载。

### 4.3 Message and source safety

底层 library error 可以作为 Rust `source` 保留，但 generic `Display` 文本不得自动进入 Skiff payload、
service error 或 external response。公开 message 必须由对应 projection 产生固定或脱敏文本。

私有源码路径、原始 JSON/BSON、secret、credential、recoverable expected plan、runtime-local address、
artifact filesystem path 和任意用户 payload 只允许进入经过权限和限长约束的诊断面。

## 5. Generated Rust-to-Skiff projection

### 5.1 Explicit opt-in

Rust error 默认只有 diagnostic capability。只有 operation semantic adapter 能构造、且在 canonical
platform error projection catalog 中显式声明的 generated DTO，才具备生成 Skiff error value 的能力。

每个 projection declaration 必须固定：

- projection key；它必须逐字等于 exact canonical public symbol，禁止任何版本后缀；
- 与该 key 相同的 exact Skiff nominal type identity；
- Rust producer family 与生成式 projection DTO；
- public message 与其它 schema-declared fields 的 sanitization policy；
- service envelope kind 与 local/wire failure 的 fallback。

Public field set、字段类型和顺序不在 projection catalog 中再抄一份；它们只从 exact Skiff type
解析后的 canonical type IR 取得。

不能仅凭 `RuntimeErrorPayload.code`、Rust type name、`Display` 文本或 JSON shape 猜测
Skiff identity。

### 5.2 Compiler and codegen ownership

Skiff `.skiff` type declaration及 `std/api.yml` public surface 是公开 symbol 与字段 schema 的唯一
事实源。Compiler-owned projection catalog 只声明 canonical public symbol key及
producer/message/wire/fallback policy；`projectionKey` 与该 symbol 必须逐字相等，它不得复制 public 字段列表
或字段类型，也不得另造带版本后缀的 wire token。

唯一 generator 必须先用 compiler 解析 public type 得到 canonical type IR，再把 type IR 与 catalog
关联生成：

- Rust projection key、typed payload DTO 和字段 encoder；
- compiler/linker 使用的 exact catch identity 和 type/materialization plan；
- service `PlatformError` payload codec；
- schema fingerprint、registry descriptor 和 surface 一致性检查。

Runtime producer 不得在 boundary、native、eval、host、request 中分别手写字段名 JSON 和
symbol string。若 Rust internal error 与 public error 字段完全一致，字段 projection 由 derive/codegen
直接生成；若必须脱敏或归并内部状态，应先在语义 owner 处转换成生成的 typed projection DTO，之后的
materialization 仍完全生成。

Compiler 无法隐式反射任意 Rust struct。需要脱敏或归并的 producer 只允许构造生成的 typed DTO；DTO
字段及其 Skiff materializer 仍来自 canonical type IR。Repository 使用 checked-in generated source：一个
generator 同时更新 compiler/runtime 共享 descriptor 与 Rust DTO，并提供 `--check` gate；各 Cargo crate
不得在 `OUT_DIR` 中各自解析 catalog 或生成另一份 registry。

Generator 输出的 `PlatformErrorProjectionRegistryRef` 是 compiler-owned generated singleton。Compiler
public API、emitter 和 package projection 只能消费该 typed authority或由它产生的 validated handoff，不能让
调用方任选 `registryId`、version 或 fingerprint。测试可以构造私有 fixture，但 production compile path不得把
descriptor 当作 authoring input、CLI option 或 ambient config。

### 5.3 Registry identity and evolution

Generator 必须按 repository canonical JSON 与 lowercase SHA-256 精确计算每个 entry fingerprint。Entry
preimage 没有可选字段，固定为：

```text
entryBytes = canonicalJson({
  schema: "skiff-platform-error-projection-entry-v1",
  projectionKey,
  nominalIdentity,
  canonicalPublicTypeIr,
  codecVersion,
  producerFamily,
  semanticAdapterOwner,
  publicMessagePolicy,
  envelopeKind,
  fallbackPolicy,
})
entryFingerprint = "sha256:" + lowercaseHex(SHA-256(entryBytes))
```

`canonicalPublicTypeIr` 是 compiler 解析 public symbol 后得到的 path/address-free、normalized 完整 IR，不能
由 catalog 复制字段或由文件路径、源码地址补事实。Whole-registry preimage 固定为：

```text
registryBytes = canonicalJson({
  schema: "skiff-platform-error-projection-registry-v1",
  registryId: "skiff-platform-error-projections",
  registryVersion: 1,
  entries: [{ projectionKey, entryFingerprint }],
})
fingerprint = "sha256:" + lowercaseHex(SHA-256(registryBytes))
```

`entries` 必须按 `projectionKey` ASCII 严格升序且 key 唯一；一个registry中出现的每个key恰有一个active
entry，不得重复。Schema、codec或上述policy输入变化时，key保持canonical public symbol不变，但必须产生新的
`entryFingerprint`与whole-registry `fingerprint`。

Canonical registry descriptor 固定为：

```text
PlatformErrorProjectionRegistryRef {
  registryId: "skiff-platform-error-projections",
  registryVersion: 1,
  fingerprint: "sha256:<64 lowercase hex>",
}
```

`registryVersion`表示descriptor/fingerprint algorithm version；entry集合或entry fingerprint变化只改变whole
registry fingerprint，不提升registryVersion。所有schema
中的 exact JSON field name 都是 `platformErrorProjectionRegistry`。M3 hard cut 把该 descriptor 作为 bytecode
header 的第六个 semantic authority，并在每个 PackageArtifact v15 root保存同一 exact value。它进入 bytecode
identity 与 Package build identity preimage，但不进入 Package Local ABI 或 ServiceProtocol identity。

- Compiler 把 whole-registry descriptor 固定进每个 PackageArtifact / bytecode header；这不是“只有实际引用
  platform error 的 artifact 才携带”的可选 pin。
- 一个 deployment exact PackageArtifact closure 中的 descriptor 必须唯一一致；mixed-fingerprint closure在
  routing view构造或Runtime load时fail closed。Strict routing view向Router提供等价于
  `{ buildId, registryDescriptor }` 的typed authority，Router不因此解析Package executable。
- Runtime binary 只加载 descriptor 与自身 generated singleton一致的 artifact，并在
  `runtime.capabilities.capabilities.platformErrorProjectionRegistry` 声明该 exact descriptor；Router保存它，
  且所有HTTP、WebSocket、Actor和task runtime dispatch都只能选择descriptor与build routing authority
  exact-match的session。Runtime loader仍对PackageArtifact、bytecode和binary做最终三方验证。
- Descriptor在同一Runtime WebSocket session内不可变化。Capabilities refresh可以重复同一 exact value；冲突
  refresh必须终止该session。更换fingerprint只能建立新的session incarnation，不能原地改写registration facts。
- Service `PlatformError` carrier 替换 closed `builtinErrorIdentity`，exact shape 为
  `{ projectionKey, entryFingerprint, encodedPayload, traceId, errorId }`。`projectionKey` 必须逐字等于
  generated entry 的 canonical public symbol，同时匹配 ASCII token grammar（`[A-Za-z0-9._-]`，1–128
  bytes）；禁止任何版本后缀。entry fingerprint 必须是
  `sha256:<64 lowercase hex>`，encoded payload 必须非空且不超过 64 KiB；outer object strict
  deny-unknown-fields 并继续使用 canonical correlation validation。Request-contract owner验证outer字段、
  bounds、correlation和canonical bytes；只有raw bytes与该validated envelope的canonical re-encoding完全相同
  才能固定为opaque carrier。只有当前registry中存在exact `(projectionKey, entryFingerprint)` pair才是
  known entry；同key不同fingerprint与完全未知key一样都是unknown-valid opaque cause，本地不可匹配但可原样
  继续转发，禁止拿当前key的codec或JSON shape猜测其schema。
- Exact known pair的payload不满足generated codec时，这是protocol failure，不得把畸形
  wire 伪造成 provider `InternalError`。Known entry的identity-specific payload validation由generated codec
  owner执行；provider/local encode在envelope固定前失败时才fallback为固定`InternalError`。

Projection key标识semantic family且永远等于canonical public symbol。Schema、codec、message、producer、
semantic adapter、envelope或fallback policy不得通过带版本后缀的key来版本化；它们变化时保留key、生成新
entry与whole-registry fingerprint，并对artifact/runtime执行显式全栈hard cut。Rolling coexistence依靠exact
pair判定与unknown-valid bounded opaque forwarding，而不是靠key-only lookup或string/JSON shape猜测旧schema。
Skiff 尚未发布，首次迁移可以直接hard cut当前手写registry，不保留其兼容reader。

首次 catalog 固定为以下21个 canonical public symbol key（按ASCII升序）；
`std.service.InternalError`是fixed fallback，`std.resource.ResourceError`是Package-owned public typed error，
二者都不进入registry；尚未激活的`std.recoverable.BoundaryError`也不计入首次catalog：

```text
config.DecodeError
std.actor.ActivationTimeoutError
std.actor.MethodInvocationTimeoutError
std.bytes.DecodeError
std.collection.ArrayIndexOutOfBoundsError
std.collection.JsonObjectPropertyNotFoundError
std.collection.MapKeyNotFoundError
std.db.ConflictError
std.db.ConstraintError
std.db.DecodeError
std.error.InstructionLimitExceededError
std.error.TimeoutError
std.file.FileError
std.http.HttpError
std.http.RequestTimeoutError
std.json.DecodeError
std.number.DecodeError
std.service.ProtocolError
std.service.ProviderUnavailableError
std.time.DecodeError
std.websocket.WebSocketRequestError
```

platform error projection registry 首次落地时已经完成以下 version hard cut：bytecode schema `skiff-bytecode-v6` →
`skiff-bytecode-v7`；bytecode identity generation 4 → 5、marker `skiff-bytecode-artifact-v4` →
`skiff-bytecode-artifact-v5`、prefix `skiff-bytecode-image-v4:sha256` →
`skiff-bytecode-image-v5:sha256`；PackageArtifact schema `skiff-package-artifact-v14` →
`skiff-package-artifact-v15`；Package build preimage marker
`skiff-package-artifact-build-identity-v12` → `skiff-package-artifact-build-identity-v13`，Package build prefix
`skiff-package-build-v13:sha256` → `skiff-package-build-v14:sha256`；runtime frame `skiff-runtime-frame-v4` →
`skiff-runtime-frame-v5`。`runtime.capabilities` 在 v5 metadata 中携带上述 registry descriptor；Router 保存
它并与 build 的 PackageArtifact descriptor exact-match 后才可 dispatch。旧 frame、缺失 descriptor、
PackageArtifact/bytecode descriptor不一致都 strict reject，不提供 dual reader。

该 registry cut 本身没有改变当时的 `skiff-bytecode-isa-v4` opcode/operand contract。随后 Phase 5 hard cut 已将
当前唯一可接受 envelope 从 bytecode schema `skiff-bytecode-v7` / ISA `skiff-bytecode-isa-v4` 升级为
`skiff-bytecode-v8` / `skiff-bytecode-isa-v5`，增加 `TakeDenseField` opcode 与 shape row 上的显式
`privilegedAffineComposite` identity；bytecode identity generation、marker 与 prefix仍是上述 v5。v7/v4 不保留
reader或兼容路径。Package Local ABI和ServiceProtocol的schema/identity generation不因这两次 registry/ISA cut
而升级。

这些 carrier/version 变化不能只由本文单独落地。M3 canonical docs cutover已经同步更新
[`bytecode-vm.md`](bytecode-vm.md) §3.1 与
[`package-service-contract-deployment.md`](package-service-contract-deployment.md) §3/§5.1/§6.3，以及
[`runtime-lazy-load-deployment.md`](runtime-lazy-load-deployment.md)和[`router-rust.md`](router-rust.md)的
session/routing contract；Phase 5 schema/ISA cut再同步更新 `bytecode-vm.md` §3.1与schema/code。canonical contracts
与generated schema必须持续一致，后续里程碑不得补写或回退这些决定。

本文当前 canonical checkpoint 位于两次 production hard cut 之后。当前仓库必须同时使用
`skiff-bytecode-v8`、`skiff-bytecode-isa-v5`、PackageArtifact v15 与 runtime-frame-v5，并携带六个 required
authority和 Phase 5 privileged affine carrier；任何 v6/v7、ISA v4、五authority或 runtime-frame-v4 的 production
路径都是缺陷，不是允许的 implementation lag 或 dual-format 协议。

Rolling runtime upgrade 期间，不同 registry fingerprint 的 runtime session 可以并存；release pointer仍
引用旧 fingerprint artifact 时，operator不得先清退最后一个 matching runtime。旧 registry/runtime 的回收
条件是没有可路由 release/artifact 再引用它，而不是新 binary 已经启动。跨 fingerprint service call仍按
上述 per-entry opaque规则传递未知 platform error。

### 5.4 Projection capability is not automatic exposure

生成 projection 只证明“能够安全构造哪个 Skiff value”。实际 failure 是否投影，由发生位置和
operation policy 决定。Runtime 只有在以下条件全部满足时才能创建 `UserException`：

1. 当前存在 active Skiff request execution context。
2. Runtime 签发的 continuation guard 仍证明 request generation、lane / resume ownership 和 call site
   都是 active；只有 source site 与 request heap 不足以证明这一点。
3. Source operation 的 semantic adapter 已把具体 failure class / phase 转成 exact generated projection
   DTO；不能由底层 Rust error type 自报 public identity。
4. Closed operation policy 显式准入 `(operation, projection key, semantic failure class, phase)`；default
   是 deny。
5. 当前结果已经进入该 operation contract 定义的 error outcome，不是 pending work、timeout loser、
   late response 或 internal cancellation carrier。
6. Projected error 与 operation contract 如实表达 effect certainty。可捕获错误可以表示远端 effect
   已发生、未发生或 outcome unknown；`catch` 只允许处理失败，不承诺 rollback、幂等或安全重试。
7. Public payload 已通过 generated schema validation 和脱敏规则。

任一条件不满足时，不得为了“让业务有机会处理”而伪造 catchable exception。

`task.submit` 是显式特例：业务 continuation 无法继续持有同一 submission context / TaskId 完成
ambiguous-acceptance 恢复，因此其 definite rejection 与 outcome unknown 按 durable task contract 都不可
捕获。HTTP primitive与Actor method/activation timeout只在原caller continuation仍active时按closed admission
投影；它们都表示outcome unknown且不触发自动重试。Service call与WebSocket没有独立primitive timeout type；
词法scope deadline由`std.error.TimeoutError`表达，request/root/inherited deadline保持terminal。任何远端
transport failure即使可捕获，也不因catchability获得rollback或safe-retry语义。

### 5.5 Timeout and instruction-limit ownership

Public timeout/budget projection固定为：

- `std.error.TimeoutError { timeoutMs: integer }`只属于词法`timeout(...)`scope；短名`TimeoutError`绑定该
  canonical symbol。只有被终止scope之外仍active的continuation可catch。
- `std.error.InstructionLimitExceededError { instructionCount: integer, limit: integer }`由hard instruction
  budget owner产生。耗尽budget的同一frame不可catch或继续执行；错误到达request root后可以固定为typed
  `PlatformError` carrier，供仍active的remote service caller在其call site按admission捕获。
- `std.http.RequestTimeoutError { timeoutMs: integer }`只表示HTTP primitive operation timeout；
  `std.actor.MethodInvocationTimeoutError { timeoutMs: integer }`与
  `std.actor.ActivationTimeoutError { timeoutMs: integer }`分别只表示对应Actor primitive timeout。三者只在
  caller continuation仍active时投影，effect/outcome均视为unknown，runtime不得自动retry。
- 若current lexical scope deadline先到，必须选择`std.error.TimeoutError`，不能伪装成HTTP或Actor primitive
  timeout。若request/root/inherited deadline先到，只形成request terminal，不向dying frame注入任何error。
- WebSocket没有独立primitive timeout type：local lexical owner仍使用`std.error.TimeoutError`；继承或root
  deadline只形成terminal。
- Cancellation，以及task、lease、idle、handshake、drain、Router ingress、load/preload等control timeout都不
  进入projection registry。

### 5.6 Failure site matrix

| Failure site | Skiff projection rule |
| --- | --- |
| Runtime/router 启动、artifact load、deployment admission、activation/control | 无 active Skiff frame；只能形成 control failure 和 operator diagnostic。 |
| External ingress 在 handler 之前 | 业务尚未运行；按 gateway protocol 返回，不能进入业务 `catch`。 |
| Handler / function 正在执行 | 只有 continuation guard 有效、semantic class/phase 被 operation 准入且 projection 不夸大 effect certainty 时才能投影。 |
| Skiff 发起的 std/native/service call site | 由该 operation 的 projection allowlist 决定；底层 source error 类型本身不决定。 |
| Handler 返回后的 response encode / egress | Provider 已无可继续执行的 catch frame；不能回投 provider。Caller 是否得到 typed error 由 service/gateway contract 决定。 |
| Scheduler、retention、telemetry、queue scan 等后台平台逻辑 | 不属于原业务 request；形成 background/control terminal，不回投已结束 request。 |
| Durable task body 内的普通 Skiff 执行 | 只在该 attempt 当前 active frame 内按普通规则 catch；scheduler/lease/settlement failure 不回投 task body。 |
| Internal cancellation / scope stop | 始终是 internal terminal，不投影。 |

“发生在 request 生命周期中”只是必要条件，不是充分条件；真正的投影点必须还有一个 runtime-owned、
仍然有效的 Skiff continuation guard。Timeout winner、concurrent loser、已结束 stream consumer 和 late
service/host response 都不能借仍存活的 request heap 重新创建 exception。

## 6. Deep source error conversion

底层 Rust library error 不携带 Skiff public semantics。只有知道 source operation 意图的 adapter
才能把它转换成 projectable typed error。

JSON 规则固定为：

| Source operation | `serde_json::Error` conversion |
| --- | --- |
| Skiff 直接调用 `std.json.decode` / typed JSON API | 转成专门的 std JSON decode error，允许该调用点投影。 |
| Recoverable envelope encode/decode | 转成 `RecoverableBoundaryError` 的对应 code；不伪装成 `std.json.DecodeError`。 |
| Artifact/config/runtime protocol decode | 转成 `InvalidArtifact`、protocol 或 internal diagnostic failure。 |
| External ingress 在 handler 前 decode | 转成 gateway protocol response；不能进入 Skiff catch。 |
| Service/provider 内部深层 JSON 使用 | 由最近的语义 owner 处理或转换；generic `serde_json::Error` 不具备 Skiff projection。 |

相同规则适用于 BSON、HTTP parser、filesystem、database driver 和其它 library error：source type
可以保留在 Rust error chain 中，public type 由 operation 语义决定。

## 7. Recoverable boundary error

### 7.1 Rust type

`RecoverableBoundaryError` 是专门 Rust error type，至少保存：

- typed `RecoverableBoundaryErrorCode`；
- diagnostic message；
- codec / trust / expected-plan 等受限诊断所需上下文。

Runtime 可以按整个类型分类；不要求业务代码理解每个内部字段。体积较大的 context、expected plan
和 detail 应装箱在该 error 内部，使 boundary/native/eval/host 的外层 error enum 不因嵌入它而扩大。

`RecoverableBoundaryErrorCode` 的内部细分可以用于 telemetry 和 runtime policy，但不能自动成为
Skiff catch identity。

### 7.2 Skiff projection

`RecoverableBoundaryError` 整个 Rust type 不直接实现 projectable capability。Recoverable semantic
adapter 必须先根据 source operation、failure phase 和 trust/effect contract 把一次 failure 分成：

- **projectable value rejection**：当前 operation contract 允许交给业务处理的 recoverable-value
  拒绝；adapter 可以构造 generated public DTO；
- **platform integrity failure**：artifact unavailable、损坏 state/sealed payload、runtime adapter/image
  不一致等平台完整性失败；保持 internal/platform terminal。

不能只按 `RecoverableBoundaryErrorCode` 建全局 allowlist。同一个 code 在不同 phase 可能代表不同 owner；
只有 operation semantic adapter 能创建 projectable DTO，底层 codec 和 error producer 都不能创建。

为将来确有 production operation 需要捕获整个 recoverable-value 拒绝类别时，保留以下 public projection
contract：

```skiff
type BoundaryError {
  message: string,
}
```

其 public symbol 是 `std.recoverable.BoundaryError`。该类型在第一个 production operation 明确准入前
不加入当前 std public surface；architecture reservation 本身不激活可捕获行为。首次激活必须在同一
change 中更新 `std` source、`std/api.yml`、reference contract、projection catalog 和 operation admission。

Public payload 只有脱敏 `message`。内部 `RecoverableBoundaryErrorCode` 仍是 bounded diagnostic
attribute，不进入 public string field；否则代码会事实上依赖这个所谓“仅诊断”字段。若未来某类 failure
需要不同的程序处理 contract，应新增专门 public error type 和 typed fields，而不是扩张一个公开 code
字符串协议。

`nodePath`、`boundaryKind`、expected plan、raw detail、`traceId`、`errorId` 和 stack 不属于
`BoundaryError` payload：

- `nodePath` / `boundaryKind` / expected / raw detail 只进入受限 diagnostic；
- `traceId` / `errorId` 由 exception/service envelope 承载；
- stack 由 `Exception<std.recoverable.BoundaryError>` 承载。

该 projection 由 compiler-owned declaration/codegen 生成，不在 eval 新增专用字段 materializer。

### 7.3 Admission

Recoverable codec 本身不具备 projection capability。每个调用 codec 的 source operation 必须在自己的
reference/architecture contract 中，按 semantic failure class 和 phase 显式选择：

- `catchable-with-contract`：operation 已定义该 error outcome、effect certainty 和可继续语义，并可在
  active call site 投影；或
- `platform-fatal`：failure 终止当前 request/attempt，只记录诊断；或
- `protocol-rejection`：failure 属 ingress/remote protocol，不进入本地 catch；或
- `background-terminal`：failure 属 durable/control/background owner。

没有声明等同于 `platform-fatal`。同一个 `RecoverableBoundaryErrorCode` 在不同 operation 中可以有不同
admission，因为可捕获性由 operation 的 continuation/effect 语义决定，不由 codec code 单独决定。

`task.submit` 当前 contract 是 `platform-fatal`；其 failure 不投影给提交业务代码。External ingress
handler 前是 `protocol-rejection`。Control/background recoverable failure 是
`background-terminal`。未来 DB、materialization 或显式 service recoverable slot 若要允许业务 catch，
必须先为具体 semantic class/phase 标记 `catchable-with-contract`，明确 effect 是否可能已经发生或
outcome unknown，并保证 continuation guard 仍有效。不得因为某个 operation 准入 value rejection，就把
该 operation 中所有 recoverable integrity failure 一并准入。

### 7.4 Cross-service behavior

一个 admitted recoverable failure 在 provider active call site 被投影成 `UserException` 后，未捕获时按
现有 canonical service error channel 导出为 exact `PlatformError`。Caller 在自己的 service call site
materialize 同一 `std.recoverable.BoundaryError` identity，并建立自己的 exception stack。

未在 active call site 投影的 provider-internal recoverable failure不能在 service export 时仅凭 Rust type
“补投影”；它按 internal failure 脱敏为 `std.service.InternalError`。这避免把 provider 私有 codec、DB
或 artifact 实现细节自动变成远端 API。

Provider 在 envelope 固定前无法编码 generated payload 时，exporter fallback 到固定 `InternalError`，
不能退回 generic JSON 或发送内部 detail。Inbound fixed envelope 的处理遵守 §8.1，不把 wire decode
失败伪造成 provider `InternalError`。

## 8. Service and external boundaries

### 8.1 Service error channel

Service channel 必须区分 producer/local failure 与已经收到的 fixed wire cause：

- 已经是 `UserException` 的 generated platform error 使用 exact `PlatformError` envelope；公开且
  schema-closed 的 user-thrown Package type 使用 `PublicTypedError`。
- Local projection/materialization 在形成 `UserException` 前失败，是本地 internal failure；若它需要越过
  service boundary，provider exporter 生成固定 `InternalError`。Provider 已有 `UserException`、但其
  payload encode 在 envelope 固定前失败时，同样生成固定 `InternalError`。
- Outer envelope 非 canonical / 非法，或本地 registry 认识 exact `(projectionKey, entryFingerprint)` pair 但
  payload codec 校验失败，
  是 inbound protocol failure。Active caller call site 按 service operation contract 投影
  `std.service.ProtocolError`；没有有效 continuation 时形成 protocol/request terminal。它不能改写成声称
  来自 provider 的 `InternalError`。
- Outer envelope 合法、exact pair 对本地 registry 未知时，保存 bounded canonical bytes为fixed opaque
  service cause。本地 catch 不匹配；未捕获时原样转发。同key不同fingerprint仍属unknown，不得调用当前key的
  codec或按payload shape猜测。
- Outer validator由request-contract拥有：它先严格检查variant、字段、长度、token/fingerprint grammar和
  correlation，再要求raw bytes与validated value的canonical re-encoding完全相同。Generated registry codec
  只在outer validation成功后处理exact-known `(projectionKey, entryFingerprint)` pair的payload；transport不得
  复制这两层语义。
- Provider local fallback 使用当前 cause 的 canonical correlation；caller-side protocol failure 分配 caller
  本地 correlation；unknown opaque 保留已经验证的原 envelope correlation。未通过 outer validation 的
  metadata 不能进入可信 correlation 或业务日志。
- 非 projectable Rust failure、私有用户错误和不能安全保留原类型的 provider failure 使用固定
  `InternalError`。已经固定的 opaque service failure 原样转发，不重新分类。
- Provider 的完整 stack 和内部 diagnostic 留在 provider telemetry；caller 只得到自己这一跳的新栈。

### 8.2 External ingress

Gateway 在 handler 前的 decode、routing、selector 和 protocol failure永远不进入业务 catch。Handler
执行中产生并未捕获的 Skiff exception可以进入 gateway 的固定 external error projection，但 external
client 不因此成为一个 Skiff caller，也不接收任意内部 error payload。

## 9. Rust error organization

每个 crate 可以拥有自己的 `RuntimeError` / `Error` enum，但应遵守：

- internal terminal 与 ordinary error 在 API 上可区分；不能靠 magic code 判断 cancellation。
- 下层 error 语义未改变时，上层优先用 wrapper/source delegation，而不是复制全部字段 variants。
- 只有上层确实改变 retry、catch、wire、ownership 或 terminal 语义时才显式 reclassify。
- 大型 leaf error 应在 leaf type 内部装箱，而不是让每个上层 enum 各自重复 boxing 规则。
- Diagnostic formatting 与 Skiff projection 是不同能力；实现一个不自动获得另一个。
- `WirePayload.code == public symbol` 不能作为 catch projection 的证据。
- 不为统一`PlatformError` envelope而统一各crate的Rust error字段；public payload只能由exact `.skiff` type
  IR生成，不能从Rust `message`、`source`或diagnostic attributes反射。

## 10. Required invariants

实现和测试必须持续证明：

1. Internal cancellation/scope terminal没有 ordinary payload或catch identity。
2. Generic deep JSON/BSON/driver error不能因 source type 相同而自动投影。
3. 每个 generated platform error 的 Rust DTO、catch identity 和 service codec 使用 key所等同canonical
   public symbol解析出的同一Skiff type IR；catalog不复制字段schema，字段增删会由generator `--check`拒绝
   漂移并生成same-key/new-fingerprint hard cut。
4. Artifact、runtime session 和 generated registry fingerprint 不匹配时不能执行该 artifact。
5. 没有有效 continuation guard 时，即使已有 request heap/site 或 projectable DTO 也不能构造
   `UserException`。
6. 未按 operation + semantic class + phase 准入的 recoverable failure 保持不可捕获；integrity failure
   不能因相同 Rust type/code 被一起准入。
7. Catchable error 如实保留 operation contract 的 effect certainty；catchability 本身不授予 rollback 或
   safe-retry 语义。
8. Public payload 不含 diagnostic-only 字段、原始 source message 或私有 stack；recoverable internal
   code 不成为 public string protocol。
9. Local/provider encode failure、inbound malformed exact-known-pair payload 和 well-formed unknown-pair projection
   分别走 internal、protocol、opaque 三条互斥路径。
10. 跨 service 保留 exact identity/correlation；本地未知但合法的 projection 可 bounded opaque forward。
11. Opcode/bytecode/VM 等 producer 与 Rust boundary producer 一样只能使用 generated projection key 和
    payload builder，不能保存手写 symbol。
12. `RuntimeErrorPayload` 始终只是 diagnostic/control DTO，不成为 universal Skiff error value。

## 11. Related contracts

- [`recoverable-value.md`](recoverable-value.md)：可恢复值、boundary context、fail-closed codec。
- [`package-service-contract-deployment.md`](package-service-contract-deployment.md)：开放 service error
  channel、fixed envelope、跨 service stack/correlation。
- [`durable-task-dispatch.md`](durable-task-dispatch.md)：task submission、ambiguous acceptance 和
  platform-fatal 规则。
- [`bytecode-vm.md`](bytecode-vm.md)：artifact/runtime registry fingerprint、opcode contract 和 VM
  fail-closed load。
- [`../reference/runtime.md`](../reference/runtime.md)：`throw` / `catch` / `rethrow`、request terminal
  和 ingress 语义。
- [`../reference/std-surface.md`](../reference/std-surface.md)：标准平台错误 public surface。
- [`../reference/observability.md`](../reference/observability.md)：受限 diagnostic、stack 和 correlation。
- [`../implementation/runtime-error-to-skiff.md`](../implementation/runtime-error-to-skiff.md)：从当前
  手写 projection 到本文模型的实施计划。
