# P5-F440M External manifest identity / deployment follower result

状态：`PASS / M0+M1_SHARED_CHECKPOINT_GREEN`。F440H 新增的 external-manifest /
`websocketJsonRpc` vocabulary 已由 artifact identity、deployment validation 与 projection follower
完整消费；没有修改 Router、Runtime、fixture、service root、integration、stable 或 live 状态。

## 1. 输入、提交与写集

| 项目 | Commit | Tree |
| --- | --- | --- |
| F440H implementation | `8933e08f722c0a201ade6c444834ad360f97ac87` | `9b418a4a59bf12783f26c0e4949c12fe687df827` |
| F440H result | `6999479c05772d95bf667d5d097acb40c898f9cd` | `f7f56658615ebaec889742c9bc026ed73a5b26aa` |
| F440M dispatch HEAD | `7c72e50bad40579504a29820e7e676d69272f531` | `6ffe6396d4c99aca93faf113f053c060edaec12f` |
| F440M implementation | `b0ae32afbe5e50bd22b595ee0dba1c37f106ad5e` | `ebdf51a3cb6c0f858a5e1123e280ef943328811c` |

Implementation 修改：

- `artifact-identity/src/{gateway.rs,constants.rs,error.rs}`
- `artifact-identity/src/deployment/validation.rs`
- `artifact-identity/src/tests/{gateway.rs,deployment.rs,mod.rs}`
- `artifact-model/src/compile_identity.rs`
- `deployment/src/projection/tests.rs`
- `compiler/tests/websocket_ingress.rs` 中唯一获授权的 prefix 断言

`artifact-identity/src/error.rs` 仅修改
`InvalidGatewayEntryIdentity` 诊断，并按 integration task amendment `beee1253` 复用
`GATEWAY_ENTRY_IDENTITY_PREFIX`，不再复制版本字符串。其余修改均在原唯一写集内。

## 2. Test-first red

修改 production 前运行：

```bash
cargo check -p skiff-compiler
```

精确复现 F440H 留下的四个 `E0004`：

| 文件 | 未覆盖分支 |
| --- | --- |
| `artifact-identity/src/deployment/validation.rs:277` | `&GatewayProtocolSurface::WebSocketJsonRpc(_)` |
| `artifact-identity/src/gateway.rs:54` | owned `GatewayProtocolSurface::WebSocketJsonRpc(_)` |
| `artifact-identity/src/gateway.rs:347` | `&GatewayProtocolSurface::WebSocketJsonRpc(_)` |
| `artifact-identity/src/gateway.rs:397` | `GatewayAdapterKind::WebSocketJsonRpc` |

加入 direct identity/deployment tests 后首次运行
`cargo test -p skiff-artifact-identity gateway` 仍由同四个 production `E0004` 停止；
同时旧 deployment test fixture 暴露缺少必填 `rpcProfiles` 和未消费新 adapter kind。这些 red
随后都由本 leaf 的显式 match arm、strict validation 和 current-generation fixture 修复，没有 wildcard、
compatibility alias 或旧 generation fallback。

## 3. Gateway normalization 与 identity

### 3.1 WebSocket connect

- connect request/result/policy shape 必须全部精确 `v1`；
- external sources producer 按 wire name sort/dedup，最终必须精确为
  `connectRequest + connectionId`；
- downlink frame producer canonicalize 后必须精确为 `binary + text`；
- `rpcProfiles` producer canonicalize 后必须精确为
  `[jsonrpc-2.0-text]`；
- loaded artifact 的乱序、重复、空 profile/source/frame sequence 不会被静默修复；
  unknown profile 在 strict serde boundary 失败。

### 3.2 WebSocket JSON-RPC

- profile 只允许 `jsonrpc-2.0-text`，dispatch 只允许 `unary`；
- source set canonical sort/dedup，必须包含一次 `websocket.jsonRpcParams`，只可另含
  `websocket.connectionId`、`websocket.businessIdentity`；
- params schema 与 result schema 都走既有 canonical external-schema normalization；
- params 的所有合法顶层值必须是 record/array，或仅由二者组成的 closed union；
  null、nullable、scalar 和混入 scalar 的 union 均拒绝；
- result schema 可为任意 canonical executable external schema，`Null` 明确合法；
- fixed/v1 external error projection 仍由共同外层验证。

loaded artifact 的 source 顺序、重复 source、required 顺序、union/nullability 等非 canonical
表示由“normalize 后必须与原值相等”的 reader boundary 拒绝。HTTP surface 同时显式拒绝
`websocketJsonRpc` adapter kind。

### 3.3 Preimage 与 generation

| 域 | 旧值 | 当前值 |
| --- | --- | --- |
| GatewayEntry schema marker | `skiff-gateway-entry-identity-v1` | `skiff-gateway-entry-identity-v2` |
| GatewayEntry prefix | `skiff-gateway-entry-v1:sha256` | `skiff-gateway-entry-v2:sha256` |
| DeploymentArtifact schema marker | `skiff-deployment-artifact-identity-v2` | `skiff-deployment-artifact-identity-v3` |
| DeploymentArtifact prefix | `skiff-deployment-artifact-v2:sha256` | `skiff-deployment-artifact-v3:sha256` |
| ServiceDeploymentInput schema | v3 | F440H 已冻结的 v4 |
| ServiceDeployment schema | v2 | F440H 已冻结的 v3 |

当前 exact gateway goldens：

- typed HTTP：
  `skiff-gateway-entry-v2:sha256:1ce33a44e725ea8fdea02caa1cc874567007967e3639e9c98ddeed04de5d4f5c`
- physical WebSocket connect：
  `skiff-gateway-entry-v2:sha256:f385624021966bab998385e1fd2c88804b51992f15f9c9d76c05d3e17a75018d`
- direct JSON-RPC method fixture：
  `skiff-gateway-entry-v2:sha256:76fd205e35d35474a2082dd58b914b25b653eeecbfd8b6c96c52d3d070eae331`

PackageArtifact build/local ABI、ServiceProtocol、WebSocketEntryId 与 RuntimeAssembly
marker/prefix 未升级。旧 gateway prefix 在 typed parser 失败；旧 deployment prefix在 canonical
record path或 declared/computed identity验证失败。Gateway 诊断直接引用 current prefix constant。

## 4. Deployment projection / admission

validation 现在区分三个严格分支：

1. HTTP entry：
   - handler 必填；
   - selector 必须为 HTTP 且 method 必填；
   - 不得占 compiler-reserved `websocket` key；
   - surface kind与完整 adapter plan source set必须一致。
2. physical WebSocket entry：
   - key精确 `websocket`；
   - surface/plan精确 `websocketConnect`；
   - selector必须是 WebSocket、`method=None`；
   - pre/guard禁止；handler缺席时adapter args必须为空。
3. JSON-RPC method entry：
   - key不能是 `websocket`，handler必填，pre/guard禁止；
   - surface/plan精确 `websocketJsonRpc`；
   - selector必须是 WebSocket、`method=Some(non-empty)`；
   - 每个 method entry只有一个 selector，external method在同 deployment唯一；
   - host/path必须匹配同 deployment 的 physical WebSocket selector；
   - adapter plan source不能重复，canonical source set必须与 protocol surface相等。

`WebSocketEntryId` 不存在作者可篡改的 deployment 字段；它仍只由
`(serviceId, GatewayEntryKey("websocket"))` 导出。本验证通过固定 physical key、同一 deployment owner
和 host/path join建立 method关联；错误 physical key、缺失 physical entry、reserved key占用及错绑
selector均 terminal 失败。

## 5. Identity / generation 矩阵

`=` 表示 bit-identical / generation不变，`Δ` 表示 identity或其直接 deployment ref自然改变。

| 变化 | PackageArtifact | ServiceContract | method GatewayEntry | generated revision | DeploymentArtifact | RuntimeAssembly |
| --- | --- | --- | --- | --- | --- | --- |
| external method rename | `=` | `=` | `=` | `Δ` | `Δ` | `Δ` |
| WebSocket host/path rename | `=` | `=` | `=` | `Δ` | `Δ` | `Δ` |
| method key / same-shape handler / formal arg order | `=` | `=` | `=` | `Δ` | `Δ` | `Δ` |
| params/result external shape change | `=` | `=` | `Δ` | `Δ` | `Δ` | `Δ` |
| valid external source-kind set change | `=` | `=` | `Δ` | `Δ` | `Δ` | `Δ` |

证据组合：

- 本 leaf direct tests 固定 method/key/handler/adapter-order不进入 GatewayEntry preimage，但完整
  gateway map、selector、handler与adapter plan进入 DeploymentArtifact；
- 本 leaf direct tests固定 params/result/source shape进入 GatewayEntry与DeploymentArtifact；
- F440H compiler tests固定真实 `websocket.yml` method/path 与 `http.yml` selector/handler mutation：
  PackageArtifact、ServiceContract bytes不变，generated revision按上述矩阵改变；
- RuntimeAssembly identity继续包含 deployment ref与gateway ingress，因此新的 deployment identity
  自然传播，不升级 assembly generation。

## 6. Tamper 与 stale 拒绝

直接 negative matrix覆盖：

- connect profile duplicate/empty/unknown与 source/frame 非 canonical；
- JSON-RPC duplicate/missing/wrong-phase source、wrong dispatch/profile、scalar/null/nullable params；
- wrong adapter kind、surface/source set mismatch、重复 source；
- missing handler、pre/guard、method `None` / physical method `Some`；
- missing physical entry、reserved HTTP key、duplicate method、host/path mismatch；
- stale input v3、deployment v2、gateway v1 prefix、deployment artifact v2 prefix；
- gateway identity内容 tamper与 declared/computed mismatch。

这些检查都发生在 identity/storage/admission可用前；没有生成或接受旧 prefix alias。

## 7. 验证结果

任务规定的最终代码状态：

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-identity gateway` | PASS，17 passed |
| `cargo test -p skiff-artifact-identity deployment` | PASS，8 passed |
| `cargo test -p skiff-deployment` | PASS，61 passed |
| `cargo test -p skiff-compiler-input service_config` | PASS，19 passed |
| `cargo test -p skiff-compiler --test http_gateway_projection --test websocket_ingress --test generated_service_deployment` | PASS，33 passed（11 + 10 + 12） |
| `cargo check -p skiff-artifact-identity` | PASS |
| `cargo check -p skiff-deployment` | PASS |
| `cargo check -p skiff-compiler` | PASS；仅既有 unused/dead-code warnings |
| `node scripts/check-artifact-identity-single-source.mjs` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

规定的 test selectors 共执行 `138 passed / 0 failed`；其中两个 artifact-identity substring
selector有预期重叠，表中保留每条命令的原始计数。

## 8. Reverse search 与后继 blocker

规定搜索：

```bash
rg -n 'GatewayProtocolSurface::WebSocketConnect|GatewayAdapterKind::WebSocketConnect|GatewayAdapterSource::WebSocket' artifact-identity deployment
rg -n 'skiff-gateway-entry-v1|skiff-deployment-artifact-v2' artifact-model artifact-identity deployment compiler
```

结果：

- production 中所有 connect match均有显式 JSON-RPC sibling分类；没有 wildcard；
- 第二条只剩四个命名清楚的 stale-generation negative test hit：
  `artifact-identity/src/tests/gateway.rs` 两处、
  `artifact-identity/src/tests/deployment.rs` 两处；
- current generation 搜索固定 gateway v2、deployment artifact v3、deployment input v4、
  deployment v3；
- `git diff --name-only` 只列第1节写集。

后继 blockers：

1. **R0 Router/Runtime reader/admission**：Router production strict regex/decoder仍只接受
   gateway v1、deployment artifact v2，且尚未消费 `websocketJsonRpc` surface/source/selector。
   Runtime loader/target/admission也仍只有 HTTP/connect执行分支。
2. **S1/F0 fixtures与wire corpus**：cross-system runtime request/connect corpus、Router tests、
   test-runner goldens及 ecosystem smoke oracle仍含旧 generation；必须由各自 fixture/tooling owner
   在 R0 schema 落地后一次刷新。
3. **service-root migrations**：真实 split manifest与三仓 service roots仍由 S1/S2/IA/P0
   各自 owner完成；本 leaf没有写入 fixture或服务。

没有剩余 compiler-owned blocker：F440H 的四个 `E0004` 已闭合，三组 compiler focused tests全部通过。
未运行完整 verify、Router、live、instance、watch或 stable workload。

Implementation 提交后 worktree clean；本文为独立 result-only提交，提交后的最终 clean 状态由交付消息
再次确认。未 merge、rebase、push，未派子 agent。
