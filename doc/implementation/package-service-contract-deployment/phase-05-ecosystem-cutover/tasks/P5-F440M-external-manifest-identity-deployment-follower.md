# P5-F440M External manifest identity / deployment follower

状态：Ready。确定性 shared leaf；对应 F440A 冻结 DAG 的 **M1**，在 F440H shared branch上继续。

## 直接父节点

- `P5-F440A-external-manifest-owner-audit-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md`
- `P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`

需要细节时只沿这三个父节点引用向上读取。

精确 shared-branch 输入：

| 项目 | Commit | Tree |
| --- | --- | --- |
| F440H implementation | `8933e08f722c0a201ade6c444834ad360f97ac87` | `9b418a4a59bf12783f26c0e4949c12fe687df827` |
| F440H result / branch HEAD | `6999479c05772d95bf667d5d097acb40c898f9cd` | `f7f56658615ebaec889742c9bc026ed73a5b26aa` |

## 目标

消费 F440H 新增的 `websocketJsonRpc` artifact vocabulary，完成 canonical normalization、
identity preimage、deployment projection/admission与版本硬切，使 M0+M1 shared branch重新完整编译：

- connect与JSON-RPC method surface均 strict/canonical；
- selector、key、handler与adapter plan只进入 deployment，不误入 gateway identity；
- method/physical WebSocket entry的部署关联 fail closed；
- deployment input v4、deployment v3及新 identity generation只有一套解释。

## 唯一写集

M1 production/test owner：

- `artifact-identity/src/gateway.rs`
- `artifact-identity/src/deployment.rs`
- `artifact-identity/src/deployment/**`
- `artifact-identity/src/tests/**`
- `artifact-identity/src/constants.rs`
- `artifact-identity/src/lib.rs`
- `artifact-identity/tests/**`
- `deployment/**`
- `scripts/check-artifact-identity-single-source.mjs`及其直接 self-test

F440H result暴露了两个原审计漏列、但 generation hard cut 必需的精确 follower文件，本任务显式补授权：

- `artifact-model/src/compile_identity.rs`：只允许刷新
  `GATEWAY_ENTRY_IDENTITY_PREFIX`；
- `compiler/tests/websocket_ingress.rs`：只允许刷新新 prefix 的直接断言；不得改投影逻辑或其它断言。

另可新增本 leaf result。禁止修改其它 artifact-model/compiler、Router、Runtime、scripts tooling、
fixtures/service roots、其它 task/result或权威设计。不得派子 agent。

## Identity generation hard cut

F440H 已把 connect preimage加入必填 `rpcProfiles`，并新增 method preimage；旧 identity generation不能继续
解释新 preimage。原子升级：

- `GATEWAY_ENTRY_IDENTITY_SCHEMA_MARKER`：v1 → v2；
- `GATEWAY_ENTRY_IDENTITY_PREFIX`：`skiff-gateway-entry-v1:sha256` →
  `skiff-gateway-entry-v2:sha256`；
- `DEPLOYMENT_ARTIFACT_IDENTITY_SCHEMA_MARKER`：v2 → v3；
- `DEPLOYMENT_ARTIFACT_IDENTITY_PREFIX`：`skiff-deployment-artifact-v2:sha256` →
  `skiff-deployment-artifact-v3:sha256`。

`SERVICE_DEPLOYMENT_INPUT_SCHEMA_VERSION=v4`与`SERVICE_DEPLOYMENT_SCHEMA_VERSION=v3`已由F440H冻结，
本任务严格消费。PackageArtifact、Package ABI、ServiceContract、ServiceProtocol、
WebSocketEntryId与RuntimeAssembly schema/identity marker不升级；assembly值会因新的deployment ref自然变化。

不得兼容读取旧 gateway/deployment generation，不保留 alias或双 prefix parser。

## Gateway normalization与validation

1. `GatewayProtocolSurface::WebSocketConnect`：
   - shapes全部精确 v1；
   - external sources精确为 connectRequest + connectionId，canonical排序；
   - downlink精确 binary + text，canonical排序；
   - `rpcProfiles` canonical sort/dedup后必须精确为
     `[jsonrpc-2.0-text]`。
2. `GatewayProtocolSurface::WebSocketJsonRpc`：
   - profile精确 `jsonrpc-2.0-text`；
   - dispatch mode精确 unary；
   - source set canonical sort/dedup，必须恰好包含一次 params，只可另含 connectionId和
     businessIdentity；
   - params schema必须 canonical且所有合法顶层值都是object/array：record、array或仅由两者组成的
     closed union；null/nullable/scalar/untyped/open shape拒绝；
   - result schema必须 canonical，`Null`合法；
   - external error projection继续 fixed/v1。
3. `GatewayAdapterKind::WebSocketJsonRpc` 只能与该 surface和上述 source phase配对；HTTP/connect不得接受
   两个新 source，JSON-RPC不得接受 connectRequest/HTTP source。
4. Loaded artifact序列顺序、重复profile/source/union/nullability必须拒绝，不能静默normalize。

Gateway identity只取 canonical protocol surface。因此：

- method字符串、GatewayEntryKey、handler、formal param名/顺序、完整adapter plan、internal nominal
  type identity、build与policy均不进 preimage；
- structured params/result external schema、profile、source kind集合、dispatch mode进入preimage。

## Deployment validation与projection

1. compiler-owned physical entry：
   - key精确 `websocket`；
   - kind/surface精确 `websocketConnect`；
   - handler可空；空handler时adapter args必须空；
   - ingress selector为 WebSocket、`method=None`；
   - WebSocketEntryId仍由 `(serviceId, "websocket")` 导出。
2. JSON-RPC method entry：
   - key不能是reserved `websocket`；
   - kind/surface精确 `websocketJsonRpc`，handler必填；
   - ingress selector为WebSocket、`method=Some(non-empty external method)`；
   - host/path必须与同 deployment的物理 entry一致；
   - 必须关联同一 physical WebSocketEntryId/deployment owner；
   - method selector唯一，不能与connect或其它method错绑。
3. HTTP entry不能占reserved key，也不能绑定 WebSocket selector/surface。
4. Adapter plan继续完整进入 deployment identity；selector method rename改变 deployment
   revision/identity，但在其它 surface不变时不改变 method GatewayEntryIdentity。
5. tampered kind/surface/source/selector/handler/entry id、stale schema/prefix均在storage/admission前
   fail closed。

## 测试先行与验证

先原样记录 F440H 的四个 `E0004` red，再写 direct tests，至少覆盖：

- connect profiles normalization/duplicate/noncanonical/empty/wrong profile；
- JSON-RPC source set、structured params、void result、wrong dispatch/profile/source；
- selector-only method rename：GatewayEntryIdentity相等、Deployment identity不同；
- params/result shape change：GatewayEntryIdentity与Deployment identity都不同；
- handler/key/formal param顺序变化不改gateway identity，但deployment plan/handler变化改deployment；
- physical/method关联、reserved key、method None/Some、host/path/entry id tamper；
- input v4/deployment v3与旧generation拒绝；
- Package/Contract identity generation不变。

必跑：

```bash
cargo test -p skiff-artifact-identity gateway
cargo test -p skiff-artifact-identity deployment
cargo test -p skiff-deployment
cargo test -p skiff-compiler-input service_config
cargo test -p skiff-compiler --test http_gateway_projection --test websocket_ingress --test generated_service_deployment
cargo check -p skiff-artifact-identity
cargo check -p skiff-deployment
cargo check -p skiff-compiler
node scripts/check-artifact-identity-single-source.mjs
cargo fmt --all -- --check
git diff --check
```

反向搜索并逐 exhaustiveness分类：

```bash
rg -n 'GatewayProtocolSurface::WebSocketConnect|GatewayAdapterKind::WebSocketConnect|GatewayAdapterSource::WebSocket' artifact-identity deployment
rg -n 'skiff-gateway-entry-v1|skiff-deployment-artifact-v2' artifact-model artifact-identity deployment compiler
```

旧generation只允许命名清楚的negative rejection fixture/测试。

## 停止规则与交付

- 若必须修改 Router/Runtime loader、真实 fixture/service root或 M0 production，返回
  `TASK_SCOPE_EXPANDED`，不得越界。
- compiler assertion以外仍需修改 compiler tests时，精确记录为后继 S1 blocker，不得扩大。
- 不运行完整 verify、Router、live、instance或stable。

Result列出 red/green计数、四类identity边界、generation变化、stale/tamper拒绝、后继loader/fixture
blocker、reverse-search和clean状态。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440h-external-manifest-compiler`
- branch：`codex/p5-f440h-external-manifest-compiler`
- result：`P5-F440M-external-manifest-identity-deployment-follower-result.md`

Implementation 与 result 分开提交。不 merge/rebase/push。完成后保留 worktree，由主 agent整体验收
M0+M1后一次合入。
