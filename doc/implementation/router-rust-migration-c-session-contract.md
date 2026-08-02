# Router Rust Migration C-session：session/directory 冻结契约

日期：2026-08-02
状态：frozen（contract pack freeze；供 W-session / E-session 消费）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §3.2
  （`RuntimeRegistrationDirectory` 双索引、replacement/cancel/barrier）、
  §3.4（identity/fence 不可互换）、§3.5（handshake）、§3.6（disconnect 是
  cancellation + barrier，不是尽力 fanout）、§5.3（C-session lane）、§5.4
  （contract pack 必填项）、§5.5（`RegistrationFrameSink` 与 demux）、§7
  （E-session）。
- 父批次：`doc/implementation/router-rust-migration-batch-3.md`。
- 叶子执行文件：`doc/implementation/router-rust-migration-contracts-session-leaf.md`。
- 同链契约：`router-rust-migration-c-model-registration-contract.md`、
  `router-rust-migration-c-process-lifecycle-contract.md`。

冲突时以权威设计为准；本文件只冻结契约，不写 production。

## 1. 范围

冻结 connection/session task 端口、`RuntimeRegistrationDirectory`、
pre-auth 上限与握手 timeout、session cancellation token + reserved terminal
+ consumer manifest + ACK barrier + fail-stop 契约。不定义
`RuntimeAdmissionPool` 的 selection policy（C-routing-query/C-dispatch），
不定义 `RuntimeHealthLedger` 的 retained 窗口（本契约只冻结观察边界），
不定义 client WS 生命周期（C-client-lifecycle）。

## 2. Identity 与 typed 类型（§3.4，冻结）

```text
RuntimeConnectionEpoch { opaque_connection_id: String, generation: u64 }
RuntimeSessionEpoch   { replica_id: ReplicaId, connection_generation: u64 }
ReplicaId             = assembly.activation:Register.replica_id（wire 固定）
RegisteredAssemblyTuple { environment, generation, assembly, config_snapshot }
RoutableRevision      { session_epoch, registered_tuple, revision: u64 }
CancellationToken / AbortHandle（per session，共享给所有基于 session 的 pending）
ConsumerTerminalPermit（installed consumer 的 owned terminal 能力）
RuntimeSessionClosed(RuntimeSessionEpoch)（consumer terminal 帧）
```

- 禁止通用 `ConnectionGenerationFence` 或裸字符串跨 domain 传递后重新猜 owner；
  `RuntimeConnectionEpoch` 不替换 `RuntimeSessionEpoch`，两者生命周期不同
  （connection epoch 在 capabilities 前存在，session epoch 在 bind 后存在）。
- `RuntimeSessionEpoch` 只在同一 physical connection 内绑定；
  reconnect = 新 connection epoch + 新 session epoch，即使 replica_id 相同。

## 3. RuntimeRegistrationDirectory（§3.2 冻结）

### 3.1 双索引

```text
current_by_replica: ReplicaId -> RuntimeSessionEpoch
sessions_by_epoch:  RuntimeSessionEpoch -> SessionRecord
SessionRecord {
  registered_tuple: Option<RegisteredAssemblyTuple>,
  registration_revision: u64,
  cancellation: CancellationToken,
  consumer_permits: Vec<ConsumerTerminalPermit>,
  barrier: BarrierState,
}
```

### 3.2 replacement / cancel / barrier

1. 同 replica 新 connection 注册时：先标记并 cancel old epoch，再安装 new
   epoch 为 `current_by_replica`；old 的 close barrier 只能删除
   `sessions_by_epoch[old]`，绝不能删除 `current_by_replica[new]`。
2. session 发布进 directory 前，只为当前 composition 中实际安装且声明
   session state 的 consumers 取得 owned terminal permit；任一 permit 获取
   失败则拒绝 registration（session 永不发布）。
3. close protocol：
   1. session task 原子触发 cancellation token；pending 即使 mailbox 饱和也能
      观察 session terminal；
   2. 通过为 terminal 预留的 mailbox slot 向已注册 consumer 发送
      `RuntimeSessionClosed(RuntimeSessionEpoch)`；
   3. consumer exact-fence 幂等清理并 ACK；
   4. close barrier 等待全部 ACK，随后 directory 删除 exact session、writer
      drain/drop permits；
   5. delivery/ACK 超时或 reserved slot 失效 → Router 进程 fail-stop
      （非零退出）；进程重启清除全部 ephemeral state，不声称补偿一次漏掉的
      ephemeral disconnect；只有 durable activation state 在 restart 后做
      persistence reconciliation。
4. 必须测试同时关闭最大允许 session 数（`runtime.maxConcurrency`）时全部
   consumer pending 最终归零。

### 3.3 RuntimeRegistrationTransition（同 session re-register）

- 捕获 current routing epoch（`Arc<RoutingEpoch>`）；验证 register exact tuple；
  原子更新 `sessions_by_epoch[session].registered_tuple` 与
  `registration_revision`；`current_by_replica` 仍指同一 session。
- candidate query 只读取一个完整 revision。
- exact duplicate（同一 tuple 再次 register）：幂等，不 bump revision。
- stale tuple（旧 epoch / 旧 assembly / 旧 snapshot）：关闭 exact session。
- new-generation-before-epoch-swap：register tuple 匹配 pending epoch 而
  current epoch 尚未 swap → 拒绝，不发布。
- 必须测试 register update 与 admission/epoch swap 并发（一次只能看到完整
  old revision 或完整 new revision，不能混合 tuple）。

## 4. pre-auth 上限与握手 timeout

- pre-auth 连接独立总量上限：默认等于 C-config 冻结的 `runtime.maxConcurrency`
  （不新增配置键；如需 operator 覆盖属于公共 config 契约变更，先上报）。
  满时 accept 直接拒绝（`PreAuthLimitRejected`），不进入握手。
- pre-auth permit 在 registered ACK 写出后释放；timeout/disconnect/terminal
  同样释放。
- bootstrap / capabilities / register 各有独立总 deadline（默认：
  bootstrap 10s、capabilities 10s、register 30s；均为进程级常量，W-session
  可用 FakeClock 注入）；超时关闭 exact connection，不发布任何状态。

## 5. cancellation / reserved terminal / consumer manifest / barrier

### 5.1 consumer manifest

- `RouterComponents` 启动时生成静态 consumer manifest：每个 installed
  component 通过 descriptor 声明是否持有 session-keyed state 及其 terminal
  sink；最终 composition 至少包含 `RuntimeAdmissionPool`、
  `RuntimeHealthLedger`、`RequestDispatcher`、`RuntimeGenerationPinLedger`、
  `WebSocketRequestBroker`、已安装 actor session owners 和
  `ActivationCoordinator`。
- 新增 capability 原子合入 component + manifest + tests 并通过进程重启生效；
  不在运行中动态扩 manifest。未安装/无状态 sink 不占 permit。
- checker 验证任一 installed session-keyed component 都在 manifest。

### 5.2 reserved terminal

- 每个 consumer mailbox 有 control/terminal 保留容量，数据容量满时 terminal
  仍可入队；reserved slot 失效 → fail-stop。
- cancellation watcher 独立于 ordinary mailbox dequeue。

### 5.3 队列与容量

- per-session outbound queue：frame 上限 256 帧 + 字节上限 4 MiB（默认）；
  inbound 同 session：frame 上限 4096 帧 + 字节上限 1 MiB（默认）；均为进程级
  默认常量，可经 FakeClock/FakeSocket 注入测试。
- 默认值修正记录（2026-08-03，迁移驱动，root 授权）：inbound frame 默认从
  64 提高到 4096。64 帧累计预算会在长活业务 session 上过早触发
  `IngressBudgetExceeded`：Runtime 每秒 health + 业务请求/actor/spawn
  帧合计可轻易超过 64（E-actor-rust two-replica full-chain 单 session
  需数百帧）。4096 仍是有界 fail-closed；饱和/预算语义测试必须显式注入
  低预算（见 `session_budget_probe`），不得依赖进程默认值。
- writer queue 同时受 frame/byte permit 限制；owner 只 non-blocking
  reserve/`try_send`；outbound permit reserve 后随 queued item 转移给 writer；
  completion/drop 恰好释放一次；disconnect 只 close+drain。
- queue full：不等待队列接受 close frame，通过独立 abort handle 关闭 socket。

## 6. Demux / sink（§5.5 冻结）

- `RuntimeFrameDemux` 按 M0 closed registry 分发；
  `assembly.activation:Register` 由 stateless family adapter 交给
  `RegistrationFrameSink`，其它 transaction variants 交给
  `ActivationTransactionFrameSink`；registration sink 调用
  `RuntimeRegistrationTransition` 并发送 registered ACK，不调用 coordinator。
- Router→Runtime `router.bootstrap` 由 connection task 经无状态
  `RuntimeBootstrapProvider` 构造写入，不回流中央 demux state。

## 7. §5.4 contract pack 必填项

### 7.1 owner / invariant

- Owner：`RuntimeRegistrationDirectory`（session truth）、pre-auth/session
  task（physical connection + cancellation）、`RuntimeHealthLedger`
  （observation 投影）、`RegistrationFrameSink`（stateless adapter）。
- Invariant：一个 replica 只有一个 current session；cancelled session 不被
  candidate/admission 选择；barrier 未全 ACK 前 directory 不删除 exact
  session；old finalizer 不能删除 replacement；所有 consumer pending 在
  success/error/disconnect/saturation/shutdown 后归零。

### 7.2 typed inputs / outputs

- Inputs：`RegisterFrame`（→ transition）、`RuntimeSessionClosed` ACK、
  `SessionCloseRequest { session_epoch, reason }`、`AbortHandle` 触发、
  `CapabilitiesFrame`（bind）、`HealthFrame`（observation）。
- Outputs：`RuntimeSessionEpoch`、`RoutableRevision`、
  `RuntimeSessionClosed`、`PermitReleased`、`BarrierComplete`、
  `FailStop`、health snapshot。

### 7.3 capacity

- session 总数：`runtime.maxConcurrency`；pre-auth：同上默认（独立上限）；
  per-session 队列/字节预算见 §5.3；consumer manifest permit 数 =
  installed session-keyed components 数。

### 7.4 queue full

- data mailbox 满：terminal 保留 slot 仍可入队；writer queue 满：abort
  socket；ingress budget 超限：abort exact session；barrier 消息走 reserved
  slot，不受 data 满影响。

### 7.5 timeout / disconnect / replacement / shutdown terminal

- timeout：bootstrap/capabilities/register 超时 → exact terminal；barrier ACK
  超时 → fail-stop。
- disconnect：cancellation 原子触发；pending 可观察 terminal；barrier 全 ACK
  后删除；无尽力 fanout。
- replacement：old cancel+barrier 后 new 才 current；old 删除不影响 new。
- shutdown：进程 shutdown 关闭所有 session via barrier（
  C-process-lifecycle 第 6 步），其余步骤见该契约。

### 7.6 health fields

- pre-auth connections、registered sessions、cancelled sessions、
  barrier pending/acked、consumer permit held/released、mailbox
  reserved/data occupancy、writer frame/byte queue、ingress frame/byte
  budget、timeout counts by kind、fail-stop counters；health retained history
  不持有 permit/socket/eligibility。

### 7.7 fake seam

- `FakeRuntimeSocket`、`FakeClock`、`FakeConsumerManifest`、
  `FakeHealthLedger`、`FakeBarrierAck`（注入缺失/超时 ACK）；directory 参考
  模型测试在 `runtime/transport/tests/session_directory_contract.rs`。

### 7.8 real boundary probe（定义）

- 真实 Runtime WS 连接：loopback listener + 真实 runtime 客户端完成
  bootstrap/capabilities/register/ACK/health；随后断开并断言 barrier 全 ACK、
  directory 与全部 consumer pending 归零；再以同 replica 新连接注册，断言
  replacement 后 old 删除不影响 new。该 probe 由 W-session/E-session 执行，
  成为 `router-rust-session-live` 的一部分。
