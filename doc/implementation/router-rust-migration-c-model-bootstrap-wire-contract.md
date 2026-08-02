# Router Rust Migration C-model-bootstrap-wire：bootstrap wire 契约

日期：2026-08-02
状态：frozen（contract pack；供 W-model-bootstrap-wire 直接消费，不写 production）

## 引用链

- 权威设计：`doc/implementation/router-rust-migration-plan.md` §3.3（active routing 单一
  authority）、§3.5（真实 Runtime handshake）、§5.3（C-model-bootstrap-wire →
  W-model-bootstrap-wire → M-bootstrap-wire）、§5.4（contract pack 必填项）、§7 E-bootstrap。
  冲突时以权威设计为准。
- 批次文档：`doc/implementation/router-rust-migration-batch-3.md`（contracts-bootstrap 节点）。
- 叶子：`doc/implementation/router-rust-migration-contracts-bootstrap-leaf.md`。
- M0 决策：`doc/implementation/router-rust-migration-m0-decisions.md`（M0-D3：closed
  frame-family registry、direction/payload presence 规则）。

## 1. 冻结范围

本 pack 冻结 Router→Runtime 的 **bootstrap assembly/config refs**：

- `RuntimeAssemblyRef` 与 `RuntimeConfigSnapshotRef` 的 exact JSON 形态、owner、校验规则；
- `router.bootstrap` frame（`RouterBootstrapFrameHeader` /
  `RouterBootstrapActivationFrameHeader`）的 strict artifact inputs、direction、payload
  presence；
- bootstrap frame 在 §3.5 handshake 中的位置（Router 先发、pre-registration、每连接一次）；
- corpus 与 fake seam；W-model-bootstrap-wire 的交付义务（不在此实现）。

非目标：不定义 `RoutingEpoch`/`ActiveRoutingEpochStore`（归 C-bootstrap 包）；
不定义 actor projection（A0）；不定义 activation DTO（contracts-activation）；不实现
W-model-bootstrap-wire codec/consumer；不写 transport production。

## 2. 冻结类型（已存在，引用 + 冻结 corpus）

### 2.1 `RuntimeAssemblyRef`（owner：`skiff-artifact-model`）

```json
{ "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:<64 lowercase hex>" }
```

- Deserialize 时严格校验：`skiff-runtime-assembly-v3:sha256:` 前缀 + 64 个 lowercase hex；
  `deny_unknown_fields`；类型错误拒绝。
- `AssemblyIdentity` 本身是无校验 newtype，**wire/record 边界一律使用
  `RuntimeAssemblyRef`（带校验），禁止裸 `AssemblyIdentity` 跨进程传递**。

### 2.2 `RuntimeConfigSnapshotRef`（owner：`skiff-artifact-model`）

```json
{ "snapshotId": "skiff-runtime-config-snapshot-v1:<32 lowercase hex>" }
```

- Deserialize 时严格校验；`deny_unknown_fields`；`RuntimeConfigSnapshotId::parse` 是唯一
  构造入口。

### 2.3 `router.bootstrap` frame（owner：`skiff-runtime-transport`）

`RouterBootstrapFrameHeader`：

```json
{
  "schemaVersion": "skiff-runtime-frame-v3",
  "type": "router.bootstrap",
  "artifactsPath": "/absolute/normalized/path",
  "serviceDb": { "mongoUrl": "<non-empty>" },
  "http": { "maxResponseBytes": 67108864 },
  "activation": {
    "environment": "<1-200 ASCII [A-Za-z0-9._-]>",
    "generation": 7,
    "assembly": { "assemblyIdentity": "skiff-runtime-assembly-v3:sha256:<64 hex>" },
    "configSnapshot": { "snapshotId": "skiff-runtime-config-snapshot-v1:<32 hex>" }
  }
}
```

Strict artifact inputs（现有 `decode_router_bootstrap_frame_header` 已实现，本包冻结）：

- `schemaVersion` 必须等于 `RUNTIME_FRAME_SCHEMA_VERSION`；
- `type` 必须等于 `router.bootstrap`；
- `artifactsPath` 必须是以 `/` 开头的 normalized absolute path（无 `.`/`..`/尾斜杠）；
- `serviceDb.mongoUrl` 非空；
- `http.maxResponseBytes` ∈ [1, 9_007_199_254_740_991]；
- `activation.environment` 通过 `validate_activation_environment`；
- `activation.generation` ≤ `MAX_SAFE_ACTIVATION_GENERATION`；
- `activation.assembly` / `activation.configSnapshot` 通过各自 typed Deserialize 校验；
- 全部嵌套对象 `deny_unknown_fields`；`http.maxRequestBytes` 不允许出现在 bootstrap
  frame（router-only 字段）。

Direction：**RouterToRuntime 专用**（§3.5：Router 在 accept 后、Runtime capabilities 之前
发送）。Frame family 级 `Session.direction()` 为 `Either`（因包含 `runtime.capabilities`/
`runtime.health` 等反方向 frame），本 pack 冻结的是 **frame 级**方向：

| frame | 方向 | payload presence |
| --- | --- | --- |
| `router.bootstrap` | Router→Runtime | Empty（Session family 规则） |

Payload presence：`RuntimeFrameFamily::Session.payload_presence() == Empty`。canonical
`router.bootstrap` 的 payload 恒为空字节；携带非空 payload 属于协议违规。

## 3. 已冻结的既有 corpus

`cross-system-fixtures/package-service-ecosystem/runtime-bootstrap-wire.json`
（schema `{schemaVersion:1, cases:[{name,outcome,header}]}`）：

- 1 accept（canonical）+ 16 reject（missing/empty/relative/non-normalized artifactsPath、
  missing/empty mongoUrl、missing/zero/fractional/overflow maxResponseBytes、
  maxRequestBytes 入侵、unknown top-level/serviceDb field、plural artifact roots）。
- 消费测试：`skiff-runtime-transport` `protocol/tests.rs`
  `router_bootstrap_shared_corpus_has_strict_parity`。本包不修改该文件。

## 4. 本 pack 新增 corpus

`runtime/transport/testdata/router-rust-bootstrap-wire-corpus.json`
（schema `skiff-router-rust-bootstrap-wire-corpus-v1`）：

- `assemblyRefs` / `configSnapshotRefs`：ref 级正负例（合法值、错误前缀、长度错误、
  uppercase/non-hex、unknown/missing field）。
- `frames`：frame 级正负例（canonical；invalid assembly identity、invalid snapshot id、
  invalid environment、overflow generation、wrong type/schemaVersion、unknown
  activation/top-level field、missing activation）。
- `family`：冻结 frame 级 direction=`routerToRuntime`、payload presence=`empty`。

消费测试：`runtime/transport/tests/bootstrap_wire_corpus.rs`。

## 5. §5.4 pack 必填项

### 唯一 owner / invariant

- owner：`skiff-runtime-transport`（frame DTO/codec）与 `skiff-artifact-model`（ref DTO）。
- invariant：**`router.bootstrap` 是唯一同时携带
  (environment, generation, RuntimeAssemblyRef, RuntimeConfigSnapshotRef) 的
  Router→Runtime wire 表面**；任何 direction/payload presence/ref 校验失败都 fail closed，
  无 fallback、无兼容 reader。

### Typed inputs / outputs

- input：decoded typed `RouterBootstrapFrameHeader`（或 `Value` 经
  `decode_router_bootstrap_frame_header`）。
- output：`RuntimeBootstrapProvider` port（契约 port，W-model/W-bootstrap 实现）：
  `fn bootstrap_frame(&self, epoch: &RoutingEpoch) -> RouterBootstrapFrameHeader`，
  payload 恒为 `&[]`。本包不实现该 port，只冻结签名与 fake seam；`RoutingEpoch` 类型归
  C-bootstrap 包。

### Capacity / queue full

- wire 层无 mailbox/queue：bootstrap 是每连接一次的控制 frame，不进入 data mailbox。
- header 长度受 binary frame codec 的 u32 header length 上限约束（现有 `encode_binary_frame`）。
- 连接级 writer queue 容量/saturation 归 C-session/C-process-lifecycle（§3.6 保留 terminal
  slot），本包不重复定义。

### Timeout / disconnect / replacement / shutdown terminal

- 本包冻结：bootstrap 只允许在 pre-registration、capability binding 之前出现；同一连接
  第二次 `router.bootstrap`、或 bootstrap 之后继续收到 bootstrap，均属协议违规 → exact
  connection terminal（状态机归 C-session 实现）。
- 握手 deadline（bootstrap→capabilities→Register）由 C-session 拥有；本包不定义 deadline
  数值。
- disconnect/replacement/shutdown 的 terminal 语义（cancellation token、close barrier）归
  §3.6/§3.7；wire 层的贡献是：**bootstrap frame 从不重试、从不排队**，连接 terminal 即
  丢失 staged bootstrap。

### Health fields

- wire 层不新增 health 字段；bootstrap 完成状态由 connection/session epoch 状态投影
  （C-session/W-session 拥有）。health 不暴露 staged bootstrap 内容。

### Fake seam

- `RuntimeBootstrapProvider`（契约 port）：fake 实现返回固定 `RouterBootstrapFrameHeader`，
  供 W-bootstrap/session 测试；真实实现从 captured epoch 构造 header。

### 至少一条真实边界 probe

- `bootstrap_wire_corpus.rs` 对真实 `decode_router_bootstrap_frame_header` +
  `encode_binary_frame`/`decode_typed_binary_frame` 做 accept/reject 断言（codec 级真实
  边界）；既有 `router_bootstrap_shared_corpus_has_strict_parity` 是同一边界的共享 corpus
  探针。真实 socket 上的 bootstrap roundtrip 归 E-session（W-session 交付）。

## 6. W-model-bootstrap-wire 交付义务（非本包实现）

1. 消费本 corpus：valid frames 解码/编码 roundtrip 通过；全部 reject 负例 fail closed。
2. 实现 payload presence 强制：`router.bootstrap` 非空 payload 拒绝（当前
   `decode_router_bootstrap_frame_header` 只解码 header，不检查 payload；corpus 负例
   `payload-non-empty-rejected` 由 W-model 实现后翻转 `currentEnforced` 断言）。
3. 实现 `RuntimeBootstrapProvider` 并从 captured epoch 构造 header。
4. 不改变既有 cross-system corpus 字节。
