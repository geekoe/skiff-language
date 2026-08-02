# Router Rust Migration C-model-registration：handshake 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 M-registration / W-session /
`H-registration-cut` 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §3.5（真实
  Runtime handshake 合同）、§3.2（`RuntimeRegistrationDirectory`、
  `RuntimeRegistrationTransition`）、§3.4（identity/fence）、§5.3
  （C-model-registration → W-model-registration → M-registration →
  `H-registration-cut`）、§5.4（contract pack 必填项）、§7
  （E-session 依赖 M-registration + H-registration-cut）。
- 父批次：`doc/implementation/router-rust-migration-batch-3.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-session-leaf.md`。
- 同链契约：`router-rust-migration-c-session-contract.md`、
  `router-rust-migration-c-process-lifecycle-contract.md`。

冲突时以权威设计为准；本文件只冻结目标 corpus 与语义，不写 production。

## 1. 范围

冻结 §3.5 的 byte-exact handshake sequence：从 TCP/WS accept 一个
`RuntimeConnectionEpoch` 到 session 成为 registered、health 被观察为止的
帧序列、阶段门禁、strict terminal 分类，以及相应 byte-exact corpus fixture
与测试。不定义 bootstrap-wire（contracts-bootstrap）、不定义 activation
transaction DTO（contracts-activation）；`assembly.activation:Register` 只
作为 registration variant 使用，其 state owner 是
`RuntimeRegistrationDirectory`，不是 `ActivationCoordinator`。

## 2. 目标 handshake 序列（§3.5，冻结目标）

```text
1. accept RuntimeConnectionEpoch { opaque_connection_id, generation }
2. Router -> Runtime: router.bootstrap
3. Runtime -> Router: runtime.capabilities
4. bind RuntimeSessionEpoch { replica_id, connection_generation } /
   acquire installed-consumer permits
5. Runtime -> Router: assembly.activation:Register
6. RuntimeRegistrationTransition 验证 committed epoch 并 publish routable revision
7. Router -> Runtime: runtime.registered（registered ACK）
8. Runtime -> Router: runtime.health（成为 registered observation）
```

### 2.1 阶段状态机（每 connection 一个 task）

```text
Accepted -> BootstrapSent -> CapabilitiesBound -> RegisterValidated -> Registered
    ^          ^                  ^                     ^
    |          |                  |                     |
    +----------+------------------+---------------------+--> Closed(terminal)
```

- `Accepted`：只允许 outbound `router.bootstrap`；inbound 只允许
  `runtime.capabilities`（且必须在 bootstrap 已写出之后）。
- `BootstrapSent`：inbound 只允许 `runtime.capabilities`。
- `CapabilitiesBound`：replica identity 已固定；inbound 只允许
  `assembly.activation:Register`。
- `RegisterValidated`：register 已通过 `RuntimeRegistrationTransition` 验证，
  routable revision 已**pending 发布**（尚未对 admission/health 可见）；
  inbound 允许 `runtime.health`（只能作为**未注册观察**丢弃，绝不进入
  registered observation），outbound 等待 `runtime.registered`。
- `Registered`：ACK 已写出；pre-auth permit 释放；pending 发布转为 registered；
  health 帧成为 registered observation；同 session 的 post-commit re-register
  按 §3.2 transition 语义处理。

### 2.2 帧集合与 direction

| 帧 | direction | codec（canonical owner：skiff-runtime-transport） |
| --- | --- | --- |
| `router.bootstrap` | Router→Runtime | `RouterBootstrapFrameHeader` + `decode_router_bootstrap_frame_header` |
| `runtime.capabilities` | Runtime→Router | `RuntimeCapabilitiesFrameHeader` |
| `assembly.activation:Register` | Runtime→Router | `encode/decode_assembly_activation_frame` + `AssemblyActivationControl::Register` |
| `runtime.registered` | Router→Runtime | `RuntimeRegisteredFrameHeader`（registered ACK） |
| `runtime.health` | Runtime→Router | `RuntimeHealthFrameHeader` |

legacy `runtime.register`（`RuntimeRegisterFrameHeader`）**不是目标 handshake
帧**；在目标序列任意阶段出现即 strict terminal（`LegacyRegisterRejected`），
`H-registration-cut` 删除 inbound legacy `runtime.register` 后本规则成为唯一
真实行为。

### 2.3 strict terminal 分类

| terminal | 触发 | 后果 |
| --- | --- | --- |
| `WrongOrder` | 阶段门禁外读帧（health/register/capabilities 错序、业务帧 pre-bind） | 关闭 exact connection；无任何 directory 残留 |
| `IdentityChange` | capabilities 重复且 replica 不同；register 的 `replica_id` 与 capabilities 绑定不一致；health 的 `runtime_id` 与会话绑定不一致 | 关闭 exact connection；不发布 revision |
| `DuplicateRegister` | `RegisterValidated` 阶段收到第二个 register（ACK 前重复） | 关闭 exact connection；pending 发布回滚 |
| `StaleRegister` | register tuple 与 committed epoch 不 exact（旧 generation / 旧 assembly / 旧 config snapshot） | 关闭 exact session；不发布 |
| `NewGenerationBeforeEpochSwap` | register tuple 匹配 pending epoch（尚未 durable commit/swap） | 拒绝并关闭 exact session；不发布 |
| `LegacyRegisterRejected` | 任意阶段收到 legacy `runtime.register` | 关闭 exact connection |
| `BootstrapWriteFail` / `AckLoss` | outbound `router.bootstrap` / `runtime.registered` 写失败（writer queue full、socket error、disconnect） | strict terminal；**session 永不成为 registered**；pending 发布回滚；ACK 丢失不允许半注册状态 |
| `BootstrapTimeout` / `CapabilitiesTimeout` / `RegisterTimeout` | 对应握手 deadline 超时 | 关闭 exact connection；pre-auth permit 释放 |
| `Disconnect` | 握手任意阶段物理断开 | 关闭 exact connection；pending 回滚；无残留 |
| `PreAuthLimitRejected` | pre-auth 连接数达到独立上限时 accept | 拒绝新 connection（不进入握手） |

### 2.4 health 观察规则

- `runtime.health` 在 `Registered` 之前到达：不是 registered observation。
  具体地：在 `RegisterValidated` 阶段到达则丢弃并计入
  `health_before_ack`（不推进状态、不终止连接）；在更早阶段到达是
  `WrongOrder` terminal。
- 只有 ACK 写出后到达的 health 帧才进入 `RuntimeHealthLedger` 作为当前
  observation；`RuntimeHealthLedger` 不持有 permit/socket/eligibility。
- health 帧 `runtime_id` 必须等于 session 绑定 replica，否则
  `IdentityChange` terminal。

## 3. Byte-exact corpus 规格

位置：`runtime/transport/testdata/registration-handshake/`。

### 3.1 frames.json（帧目录）

```json
{
  "schemaVersion": 1,
  "corpus": "registration-handshake-v1",
  "frames": {
    "<frame-name>": {
      "direction": "RouterToRuntime | RuntimeToRouter",
      "frameType": "router.bootstrap | runtime.capabilities | assembly.activation:Register | runtime.registered | runtime.health | runtime.register",
      "decodeAs": "RouterBootstrap | Capabilities | AssemblyRegister | Registered | Health | LegacyRegister",
      "frameHex": "<完整二进制帧 hex，SKBF magic + version + encoding + 长度 + JSON header + payload>",
      "header": { "...": "typed header 语义 JSON（无字段序要求）" }
    }
  }
}
```

- `frameHex` 是本契约的 byte-exact 事实；测试用 canonical codec decode 后
  re-encode，必须逐字节相等（`encode(decode(hex)) == hex`）。
- payload：session/activation 族为空（`PayloadPresenceRule::Empty`）。

### 3.2 scenarios/*.json（序列语义）

```json
{
  "schemaVersion": 1,
  "scenario": "<name>",
  "epoch": {
    "environment": "prod",
    "generation": 42,
    "assembly": { "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:<64 hex>" },
    "configSnapshot": { "snapshotId": "skiff-runtime-config-snapshot-v1:<32 hex>" },
    "pending": null
  },
  "preAuthLimit": 2,
  "events": [
    { "kind": "accept", "connection": "c1", "connectionGeneration": 1 },
    { "kind": "write", "connection": "c1", "frame": "bootstrap.prod.42" },
    { "kind": "read", "connection": "c1", "frame": "capabilities.runtime-a" },
    { "kind": "read", "connection": "c1", "frame": "register.prod.42.a" },
    { "kind": "write", "connection": "c1", "frame": "registered.runtime-a" },
    { "kind": "read", "connection": "c1", "frame": "health.empty" }
  ],
  "expect": {
    "outcomes": { "c1": "Registered" },
    "refusedCount": 0,
    "preAuthCount": 0,
    "registeredSessions": ["runtime-a"],
    "observedHealth": 1,
    "healthBeforeAck": 0,
    "routableRegistered": true,
    "publishedPending": false,
    "revision": 1,
    "failStop": false
  }
}
```

事件 kinds：`accept`、`write`（outbound 帧）、`writeFail`（outbound 写失败，
用于 ACK 丢失）、`read`（inbound 帧）、`timeout`（`kind` 为
`bootstrap|capabilities|register`）、`disconnect`。

### 3.3 必选场景清单（测试同文件断言存在）

accept：

- `accept-sequence`

负例序列：

- `wrong-order-health-before-capabilities`
- `wrong-order-register-before-capabilities`
- `legacy-register-rejected`
- `identity-change-register-replica`
- `identity-change-capabilities-replica`
- `duplicate-register-pre-ack`
- `stale-register-old-generation`
- `tuple-mismatch-assembly`
- `new-generation-before-epoch-swap`
- `ack-loss`
- `health-before-ack-no-observation`
- `pre-auth-limit`
- `bootstrap-timeout`
- `capabilities-timeout`
- `register-timeout`
- `disconnect-mid-handshake`
- `re-register-exact-idempotent`
- `re-register-stale-after-ack`

## 4. 与当前 TS/Rust wire 的差异记录（冻结目标 corpus）

| 表面 | 当前 wire（main@1d442366） | 目标 §3.5 corpus（本契约） | 收敛动作 |
| --- | --- | --- | --- |
| registration variant | inbound legacy `runtime.register`（`RuntimeRegisterFrameHeader`，含 service/build/revision/targets 等字段） | `assembly.activation:Register`（environment/generation/assembly/config_snapshot/replica_id） | `H-registration-cut`：删除 inbound legacy `runtime.register`；`runtime.registered` 只作为成功 Register 的 ACK |
| capabilities 位置 | legacy register 帧内嵌 `capabilities` 字段；standalone `runtime.capabilities` 类型已存在 | 独立 `runtime.capabilities` 帧，绑定 replica identity | `H-registration-cut` 后 standalone capabilities 是唯一来源 |
| connection/session epoch | wire 无 epoch 字段（router-local 状态） | `RuntimeConnectionEpoch` / `RuntimeSessionEpoch` 是 router-local typed identity，不需要 wire 字段 | W-session 实现 bind |
| `runtime.registered` | 已存在，作 ACK | 不变 | M-registration |
| `router.bootstrap` / `runtime.health` | 已存在 | 不变（bootstrap-wire 内容归 contracts-bootstrap） | M-registration |

## 5. §5.4 contract pack 必填项

### 5.1 owner / invariant

- Owner：`RuntimeRegistrationDirectory`（registered tuple、session epoch、
  revision）+ 每 connection 的 pre-auth/session task（phase 状态机）+ stateless
  `RegistrationFrameSink`（decode `assembly.activation:Register` 后委托
  `RuntimeRegistrationTransition`，发送 registered ACK；不调用
  `ActivationCoordinator`）。
- Invariant：一个 session 只有一个 phase 真相；register 成功前不进入
  routable directory；ACK 写出前不产生 registered observation；任何 strict
  terminal 后该 connection 不再产生任何 directory/health/admission 效果；
  pending 发布要么在 ACK 后转正，要么在 terminal 时回滚。

### 5.2 typed inputs / outputs

- Inputs（inbound frames）：`RuntimeCapabilitiesFrameHeader`、
  `AssemblyActivationControl::Register`、`RuntimeHealthFrameHeader`；
  outbound 指令：`WriteBootstrap`、`WriteRegisteredAck`、`Abort`、`Close`。
- Outputs：`RuntimeSessionEpoch`（bind 后）、
  `RegistrationTransitionOutput { routable_revision, ack_required }`、
  `RegisteredObservation`（health，仅 ACK 后）、
  `SessionTerminal { connection_epoch, terminal_kind }`、
  `HealthBeforeAckDropped`（计数，非 observation）。

### 5.3 capacity

- pre-auth 连接独立总量上限：默认等于 C-config 冻结的
  `runtime.maxConcurrency`；到达上限时 accept 拒绝（`PreAuthLimitRejected`）。
  pre-auth permit 在 ACK 写出后释放（session 转入 registered 不再占 pre-auth
  slot）。
- session 总数上限：`runtime.maxConcurrency`（C-config，required）。
- 每 connection outbound/inbound 队列容量与字节预算见
  C-session 契约 §5.3（本 pack 只冻结上限语义：满则 abort，不接受新帧）。

### 5.4 queue full

- outbound writer queue full 或 byte budget 超限：不等待队列接受 close frame，
  通过独立 abort handle 关闭 socket；bootstrap/ACK 写失败即
  `BootstrapWriteFail` / `AckLoss` strict terminal。
- inbound 队列满或 per-session frame/byte budget 超限：abort exact session。

### 5.5 timeout / disconnect / replacement / shutdown terminal

- 握手 deadline：bootstrap、capabilities、register 各有独立总 deadline
  （默认值与 C-session 契约一致），超时对应 terminal 并释放 permit。
- disconnect：任意阶段 terminal，pending 回滚，无残留。
- replacement：同一 replica 新 connection generation 到来时，旧 session 先
  cancel + barrier（C-session），新 session 才能成为 current；handshake
  corpus 覆盖 new-generation-before-epoch-swap 拒绝。
- shutdown：进程 shutdown 时所有握手中的 connection 走 disconnect terminal
  + barrier（C-process-lifecycle 第 6 步）。

### 5.6 health fields

- per-connection handshake phase 计数（accepted/bootstrap-sent/
  capabilities-bound/register-validated/registered/closed-by-terminal-kind）。
- pre-auth 占用、registered sessions/revisions、`observedHealth`、
  `healthBeforeAck`、`ackLossCount`、`failStop` 标志。
- 日志/health 不含 Mongo URL、secret、业务 payload。

### 5.7 fake seam

- `FakeRuntimeSocket`（内存 channel reader/writer + 可注入 write 失败）、
  `FakeClock`（可推进时间触发 timeout）、`FakeHealthLedger`、
  `FakeConsumerManifest`（固定 installed-consumer 集合）。corpus 测试直接
  消费 fixtures + 参考状态机；W-session 实现必须用同一 fixtures 通过真实
  codec/state machine。

### 5.8 real boundary probe（定义，W-session/E-session 执行）

- `router-live:session` probe 定义：loopback 启动真实 listener（C-net 冻结的
  hyper + tokio-tungstenite），fake runtime 客户端按 frames.json 逐帧发送
  字节；断言 wire 上收到的 `router.bootstrap`/`runtime.registered` 与
  fixture 字节一致；随后发送负例帧（如 ACK 前 duplicate register）断言
  connection 被关闭且 directory 归零。该 probe 在 M-registration +
  W-session 后成为 `router-rust-session-live` 的 required managed CI。

## 6. H-registration-cut 依赖

- `H-registration-cut` 依赖本 pack + M-registration：current TS Router/Runtime
  必须先通过同一 shared corpus，再切真实 handshake。
- cut 后 inbound legacy `runtime.register` 被删除；`runtime.registered` 仅作
  Register ACK；standalone capabilities 是唯一 capabilities 来源。
- 本 pack 不实现也不提前消费该 cut。

