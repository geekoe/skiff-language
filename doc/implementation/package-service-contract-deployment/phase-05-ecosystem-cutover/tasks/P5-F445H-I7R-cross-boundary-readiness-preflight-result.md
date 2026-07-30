# P5-F445H-I7R cross-boundary readiness preflight result

## Outcome

状态：`READY_FOR_I7_DAG`

I7 的repository事实、真实入口、最小owner、遮挡顺序和最终gate边界已经有界冻结。当前没有
I7 implementation节点可启动：所有I7 production节点仍必须等待最终I6 acceptance。该等待是
已知父依赖，不是新的设计阻塞。

没有发现需要重开以下冻结语义的证据：

- 第一版service call只继承caller current execution deadline和外层`timeout(...)`；
- deployment `policy.timeoutMs`不是callee/dependency timeout；
- 没有公开peer cancellation、`$/cancelRequest`、`-32800`或`CancelError`；
- WebSocket普通send不挂起；
- `requestJsonToConnection(connectionId, method, value)`保持三个参数，transport id由平台隐藏；
- HTTP/WebSocket ingress分别由`http.yml`/`websocket.yml`拥有。

本预检使用了三个互不重叠的只读分片：Skiff、Internals、official packages。没有修改三个输入
repository，没有运行Cargo/Router/service完整gate，没有安装依赖，也没有访问stable、MongoDB、
OAuth、browser、外部network或live target。实际执行的命令只有Git/source检查、Skiff selector
dry-run，以及official-package Phase 05 runner的离线`--list`和`node --check`。

## Repository snapshots and provenance

### 固定Skiff输入

```text
commit  a45389c6083ddd5b57b6d2ed202c1b3816f8f468
tree    908aee0d6a95290e8ea6dae89228bf635ce6439e
parent  c8dc205dcd691d1f1108ded1f6928379563f6c00
subject docs: simplify service call timeout sources
```

`a45389c6..270f563e`只新增本任务contract；production和tests没有变化。因此下面的Skiff静态事实
同时适用于固定输入和本预检开始时的integration HEAD。

### 开始时的三个repository

| Repository | Branch | HEAD | Tree | Dirty / upstream |
| --- | --- | --- | --- | --- |
| `/Users/geek/workspace/skiff-phase-05-integration` | `codex/package-service-phase-05` | `270f563e7866746886f16f3cad301eb6bb3b7bb6` | `942ecca566a54ff10051507db1d13f66c97a033f` | clean |
| `/Users/geek/workspace/internals` | `main` | `5861c13f3a92b7fb56a5cfa689e46f5d0462a02d` | `867c99c155386299e7dbb8b4fed95cee2427ba84` | clean；`origin/main` ahead 238 |
| `/Users/geek/workspace/skiff-packages` | `main` | `5defc94161cee14def1a6bbb340308004e65b741` | `d8763acf82e0320135704297f2419bf5cd3558e5` | clean；与`origin/main`一致 |

本任务worktree开始时与Skiff integration同HEAD/tree；唯一dirty是本任务新建、尚未提交的result，
不是输入repository的既有dirty。

### 已存在但尚未进入各自`main`的Phase 05输入

| Repository | Prepared input | Relation to requested `main` | Use in I7 |
| --- | --- | --- | --- |
| Internals | `codex/package-service-phase-05@19d41001f048efc0b70e13c21d105a855ddd86e2`, tree `15c48e07cc3d51794269719c606c87169bd0ee72` | `main...branch = 0/104`，`main`是其祖先；worktree clean | Internals I7的base，不在旧`main`上重做104个提交 |
| `skiff-packages` | `codex/package-service-phase-05@19cfab5dfc827450d37e1a103d21f31f8effa4f0`, tree `44081bd0498919086c13adea97c07722cb768352` | `main...branch = 0/26`，严格fast-forward；worktree clean | official-package consumer候选 |
| Agine terminal draft | stash `91f3cc32e9d6ce0b14b4145d3d94815ab1a52420`；untracked parent `a6f0b6d418bd4a6f74af9c6dc48a94ff951c50eb` | base是Internals Phase 05 `19d41001` | 只在新的clean worktree中`apply`/materialize；不得`pop`或改写原stash |

`skiff-packages`全树没有`AGENTS.md`。已完整读取Skiff、本任务目录以及实际涉及的Internals根、
Agine client/host、Codex Relay根/service约束。

## Skiff source to Router trace

### 当前canonical chain

| 跳点 | 当前owner和真实路径 | 当前positive | I7仍缺的receipt / 遮挡 |
| --- | --- | --- | --- |
| Source/config | `compiler/input/src/service_config.rs`读取`package.yml`、`api.yml`、`service.yml`，并分别读取`http.yml`、`websocket.yml`；`compiler/driver/authoring.rs`拒绝legacy authoring | 当前split manifest可被compiler读取 | 没有tracked `.skiff` source把nested timeout和五类current-scope operation组合起来；旧inline manifest会先遮挡后续所有证据 |
| Source → File IR | `compiler/driver/pipeline/mod.rs`、`compiler/lowering/src/source_file_lowering.rs`、`compiler/lowering/src/file_ir/identity.rs` | `compiler/tests/timeout_artifact_lowering.rs`用临时source证明timeout fact和File IR identity | positive不是checked-in `.skiff`，也没有HTTP/WS/file/Actor/service call |
| Package/service artifacts | `compiler/projection/src/package_artifact/projection.rs`、`compiler/driver/authoring/package_publication.rs`、`compiler/driver/generated_deployment.rs` | package、contract、deployment和gateway entry均有typed writer | 还没有同一真实fixture的PackageArtifact、ServiceContract、DeploymentArtifact、assembly exact receipt和负向identity mutation |
| Assembly/storage | `compiler/driver/authoring.rs`、`deployment/src/assembly/mod.rs`、`deployment/src/storage/records.rs` | exact immutable local closure；无remote fallback | synthetic snapshot可绕过artifact reader，因此不能替代真实assembly receipt |
| Loader/Host admission | `runtime/loader/src/filesystem_resolver.rs`、`runtime/loader/src/runtime_assembly.rs`、`runtime/host/src/loader/assembly_admission.rs` | exact typed reference、content validation、load/link/admit/atomic publish | Host positive source是在Rust fixture中生成，不是tracked source，也不跨Router |
| Router HTTP | `router/src/router/assemblyHttpGateway.ts` → `runtime/host/src/host/router_session.rs` → `request_entry/assembly_wire.rs` | Router unit覆盖unary、status/header/body和ordered stream；Host有typed raw/stream unit | Router positive大多使用synthetic manifests/socket；没有同一个compiler artifact经Host到client的receipt |
| Router WebSocket | `router/src/gateway/webSocketGateway.ts` → JSON-RPC bridge → `runtime/host/src/host/request_entry/websocket_jsonrpc.rs` | connect和method dispatch的positive/negative已存在 | tracked source只有connect；没有source调用`requestJsonToConnection`，也没有hidden-id的source→Router receipt |
| Observable output | Router HTTP response、stream writer或WebSocket RPC result | 每层分别有unit evidence | 没有一条连续receipt覆盖tracked source → compiler/artifact → Host → Router → observable result |

### 当前identity generations

| Identity | Current generation |
| --- | --- |
| File IR / format / opcode table | `skiff-file-ir-v9` / `v7` / `v2` |
| PackageArtifact | `skiff-package-artifact-v9` |
| Package build / local ABI | `skiff-package-build-v10` / `v7` |
| ServiceContract / ServiceProtocol | `v5` / `v5` |
| ServiceDeploymentInput / ServiceDeployment / DeploymentArtifact | `v4` / `v3` / `v3` |
| GatewayEntry | `v2` |
| RuntimeAssembly | `v2` |
| Runtime frame / fixed `response.error` | `skiff-runtime-frame-v1` / `v2` |

Identity owners are `artifact-model/src/schema.rs`,
`artifact-model/src/compile_identity.rs`, `artifact-model/src/activation_lexical.rs`,
`artifact-identity/src/constants.rs` and `runtime/transport/src/protocol.rs`.

F445C的compiler fix已经存在：embedded `AnyInterface` owner identity会在Dependency和PackageId形式间
canonicalize，同时保留package/symbol/ABI/generic负向。F445C的7个focused tests证明compiler
根因已修；它没有恢复或重新编译F444C consumer，因此不能冒充Agine exact GREEN。

### Current-scope真实source矩阵

`rg -n --glob '*.skiff' '\btimeout\s*\('`在当前Skiff tracked source中是零匹配。

| Carrier | 当前真实source | 当前边界证据 | Nested timeout receipt |
| --- | --- | --- | --- |
| HTTP unary/raw stream | `runtime/live-tests/internal/http_adapter.skiff`和`runtime/live-tests/http.yml` | `streamEcho`产生start/chunks/end；live test直接调用handler | 缺；且直接调用不证明Router HTTP |
| WebSocket request | `test-runner/fixtures/package-service-websocket-smoke/main.skiff`和`websocket.yml` | 只证明connect | 缺；没有tracked `requestJsonToConnection` call |
| File | `runtime/live-tests/internal/file_live.live.test.skiff` | 真实file operation | 缺 |
| Actor | `test-runner/fixtures/actor-full-chain-acceptance/main.skiff` | 真实Actor operation | 缺 |
| Service call | `test-runner/fixtures/package-service-host/consumer/main.skiff` | 真实`payments/echo`call | 缺 |
| Timeout lowering | `compiler/tests/timeout_artifact_lowering.rs`内的临时source | exact File IR/build identity；timeout改变build但不改变public ABI | 不含任何跨边界carrier |

当前canonical in-process实现与冻结语义一致：

- `runtime/eval/src/eval_context/timeout.rs`派生lexical child current scope；
- `runtime/eval/src/assembly_execution/async_stream_cancel.rs`把current execution context带入provider；
- `current_scope.rs`和`prepared_unary.rs`只读取current cancellation/effective deadline；
- `assemblyHttpGateway.ts`的deployment timeout只属于ingress；
- legacy `ServiceTimeoutConfig`不在canonical assembly service-call path上。

### 现有positive/negative为何不能拼接成I7

- `test-runner/tests/package_service_contract_deployment.rs`可以编译真实runtime-live roots并证明current
  gateway identities，但不执行Host/Router。
- `router/tests/helpers/compilerArtifacts.ts`和
  `compilerGeneratedManifestCompatibility.test.ts`是真compiler artifact → Router reader receipt，
  但source只有HTTP且没有timeout。
- `runtime/host/src/host/router_session/tests/runtime_assembly_request.rs`执行Host positive，但source由
  Rust fixture生成且不跨Router。
- Router HTTP positive/negative在`assembly-http-gateway-stream.test.ts`和
  `runtime-assembly-unary-dispatch.test.ts`；WebSocket在
  `runtime-assembly-websocket-jsonrpc-protocol.test.ts`和
  `runtime-assembly-websocket-jsonrpc-dispatch.test.ts`。这些都是重要的下游receipt，但不能证明
  source/compiler。
- `assembly-http-gateway-stream.test.ts`中的deployment v2 synthetic snapshot会绕过current artifact
  reader；`filesystem-runtime-assembly-snapshot-loader.test.ts`明确拒绝v2。这是当前最具体的
  “下游unit通过但真实artifact失败”遮挡例。

### Wire/legacy审计

- production中没有公开peer-cancel协议；`$/cancelRequest`只在测试里作为普通显式method或被忽略的
  notification，不会取消active RPC；`-32800`零匹配。
- `CancelError`只保留在fail-closed/negative fixture。
- `requestId`、`request.cancel`和connection-request cancel是Router↔Host内部correlation和
  best-effort stop hint，不是business DTO。
- `peerRequestId`由Router protocol negative显式拒绝。
- generic legacy `websocket.receiveEvent`仍在非canonical分支；current RuntimeAssembly connect +
  JSON-RPC validation拒绝它。I7 positive不得走该分支。
- `runtime/host/src/capability_context/outbound_service.rs`和
  `runtime/eval/src/service_dispatch.rs`仍保留legacy relay实现；canonical assembly安装路径不使用它。
  I7 fixture必须反向证明没有落回该relay。
- 普通WebSocket send仍不挂起；`std/websocket.skiff`的request API仍为三个参数，transport
  correlation只存在于Host capability实现。

## Internals real consumers

### Agine identity与terminal状态

F444C报告的首个interface identity根因已经由F445C修复，但consumer没有exact revalidation：

1. F444C的四个首错是`Dependency { dependency_ref: "agent" }`与
   `PackageId { package_id: "agine.ai/agent" }`在
   `agent/tools.runtimeBindingsWithSubagent`上的owner mismatch；
2. F445C commit `f50c4a774b5e7700bf4652be749151e086ba644a`实现canonicalization并有7个
   positive/negative tests；
3. F445C没有恢复F444C stash；
4. requested Internals `main`会更早在legacy manifest的`unknown field http`停止。

所以准确结论是“compiler根因已结构性修复，Agine exact GREEN仍缺”，而不是“F444C已经通过”。

| Surface | Requested `internals/main` | Prepared phase / terminal draft | I7 gap |
| --- | --- | --- | --- |
| Manifest | `agine/service/service.yml`仍内联services/packages/http/websocket/timeout；无`package.yml`、`http.yml`、`websocket.yml` | Phase branch仍有inline raw HTTP/receive；F444C stash已拆出43个HTTP entries、connect-only `websocket.yml`，`service.yml`只留id | 在final I6/F445C上materialize stash并exact compile |
| Service WebSocket | `agine_service.skiff`同时实现connect和receive | stash只声明connect | 禁止恢复receive或增加“以后可能用”的peer business method |
| Host outbound RPC | production中零`requestJsonToConnection`；`host_file_rpc.skiff`和`host_toolprovider_connection.skiff`用DB relay + `sendJsonToConnection` | stash `host_peer_rpc.skiff`直接调用`host.files.list`、`host.files.search`、`host.current-directory` | 三参数call、hidden transport id、non-suspending send的source和runtime receipt |
| Business DTO | current `api/agine.skiff`把`requestId`嵌入file/tool-provider params | Phase `agine/protocol/hostPeer.ts`和stash `host_peer_protocol.skiff`的business params/results无`id`/`requestId` | 保留recursive JS negative和TS compile negative |
| Client uplink | chat主体HTTP，但file/tool-provider/thread/message actions仍调用generic WS request/send | Phase行为已大多迁HTTP，但`ws.ts`仍暴露unused generic send | 删除上行业务surface，WS只保留connect/下发观察 |
| Chat smoke | `/session`、chat create/send/get是HTTP；agent create和cleanup仍是WS request | 尚未terminal | 把create/cleanup迁HTTP；不要把smoke算作Host或raw relay receipt |

### Codex Relay和真实service-call hop

真实internal service call不是Codex Relay自己调用另一个service，而是AIHub调用Codex Relay：

```text
Agine chat/send
  -> agent_bridge.acceptUserMessage
  -> AgineAgentLlmClient.streamChat
  -> aihub/managedLlm.streamChat
  -> aihub_service.skiff
       codexRelay/relayProxy.responsesCompletedResult(request)
  -> codex-relay/relay.skiff
  -> llmProviders.chatgptPlan.responsesCompleted
  -> AIHub completed SSE
  -> agent/Agine WebSocket downlink
```

`aihub/service/service.yml`声明`agine.ai/codex-relay`；
`codex-relay/service/api.yml`导出`relayProxy`；
`aihub/service/internal/aihub_service.skiff`使用canonical slash service call。
因此Codex Relay任务必须带一个bounded AIHub follower；只测试Relay export会漏掉唯一真实caller。

对外server-stream是另一条路径：

```text
codex-relay HTTP route
  -> proxy_runtime.proxy
  -> API-key: std.http.stream(HttpClientRequest)
     or ChatGPT plan: llmProviders.chatgptPlan.responses(...)
  -> responses_projection
  -> std.http.streamStart / streamChunk* / streamEnd
```

当前实现可证明：

- status保留；
- 未被过滤的event/chunk相对顺序保留；
- metadata/content headers会过滤；
- SSE会buffer、filter、sanitize并可能rechunk；
- JSON body会buffer/sanitize；
- 非JSON/非SSE chunk按序转发。

所以不能宣称“所有headers和原始chunk boundaries bit-for-bit保留”。I7 hermetic contract应断言
`start(status, filtered headers) -> ordered surviving chunks/events -> end`。若后续任务要求bit-for-bit
raw proxy，那是新的设计/scope，不得在I7 consumer任务中暗改。

现有tests覆盖split chunk、UTF-8、SSE filter、JSON sanitize、local 503/404/401和package chunk，
但没有成功API-key external stream从start到end的receipt，也没有failure-before-start /
failure-after-start矩阵。Chat smoke不经过这条external route。

### Gate分类

| Gate | Isolation truth | 不得声称 |
| --- | --- | --- |
| Agine service `type-check` | 临时artifact/build root，按Codex Relay → AIHub → Agine编译；不reload stable；会准备local package store | 只证明source compile，不是Host/runtime/Router receipt |
| 当前Agine service `npm test` | Node architecture部分hermetic；旧wrapper传current Skiff已拒绝的CLI参数，且旧配置默认共享`127.0.0.1:27017` | 当前命令不是final可执行receipt，也不是纯hermetic |
| Phase worktree receipt scripts | compile source并检查artifact/receipt | 没有执行完整`.test.skiff` runtime matrix |
| Agine client `tsc`/Vitest | local | 不证明stable account、provider或browser |
| Agine Host unit/type-check | local fake/loopback | 不证明真实Host process/home/restart |
| API chat smoke | 不需要browser，但需要stable Router/runtime/Mongo、coherent artifacts和provider network/config | 不证明Host file RPC，也不证明Codex external raw stream |
| Playwright | 需要已安装Chrome和local web/stable state | 不得与API smoke计为同一receipt |
| Codex Relay Node tests | fake fetch/dependencies；部分使用ephemeral loopback | 不是real OAuth/external upstream |
| OAuth/live scripts | real account、singleton localhost callback和external network | 必须独立授权，不能放进自动hermetic test |

### Chat smoke实际跳点和最小诊断

当前`agine/client/e2e/api.chat-smoke.mjs`依次执行：

1. `POST /session`并取得cookie；
2. 可选保存provider credential；
3. WebSocket connect；
4. legacy WS `agents/create`；
5. HTTP `/chat/create`、`/chat/send`、轮询`/chat/get`；
6. WebSocket观察`chat/text-delta`；
7. legacy WS chat/agent cleanup；
8. 可选HTTP credential cleanup。

默认`hostProviders: []`，所以正常chat smoke没有Host leg。最终诊断顺序必须是：

1. stable Router/runtime health和exact loaded artifact/version；
2. `/session`；
3. authenticated `/chat/list`，隔离Router → Agine → session/Mongo；
4. connect-only `/ws`；
5. HTTP agent/create、chat/create、chat/send；
6. `/chat/get`的chat/runtime/message terminal diagnostics；
7. 完整API chat smoke；
8. 独立Host RPC probe；
9. 独立Codex Relay external stream probe。

把Host或raw stream失败塞进chat smoke只会增加遮挡，不会增加同一receipt的证明力。

## Official packages

### 两层实际dependency closure

production consumer closure是：

```text
compiler-owned skiff.run/std@1.0.0
  -> skiff.run/http-session@1.0.0
       ├─> skiff.run/track@1.0.0 -> agine.ai/api@0.1.0
       └─────────────────────────> agine.ai/api@0.1.0
```

Codex Relay只依赖Internals的`llm-providers`和`llm-api`；AIHub也不新增official root。
Skiff `package-service-host` fixture只依赖`example.com/helper`和`example.com/payments`。

另有一个必须单独标注的fixture-only edge：
`test-runner/src/canonical_package/tests/combined.rs`中的ignored F76 provenance probe显式枚举
`http-session`、`aliyunoss`、`track`、`openai`。因此：

- minimal runtime consumer gate只包含`http-session,track`；
- 若I7保留/刷新F76 real-package provenance probe，compile-only closure会额外包含
  `aliyunoss,openai`；
- 后两者不是Agine/Codex Relay生产依赖，也不在I6 current-scope关键路径上；
- 当前F76从package root发现旧测试，而Phase 05 package branch已把测试迁到`tests/*` service roots，
  所以该ignored probe本身也需要test-only provenance refresh，不能复用旧GREEN。

### Requested `skiff-packages/main`不是current consumer candidate

| Current-main gap | Evidence | Prepared Phase 05 state |
| --- | --- | --- |
| `HttpSessionSource`未从`http-session/api.yml`公开 | source引用它；已有“named child not explicitly public”首错 | `19cfab5d`已公开 |
| `http-session`、`track`有DB schema但manifest无state requirement | current compiler fail closed | prepared branch已声明state |
| Track callable仍使用旧`alias.publicPath` | current compiler只接受`alias/publicPath` | prepared branch已改slash |
| root仍有legacy `skiff.test-doubles.json` | current test runner不接受旧sidecar | prepared branch已迁到零-ingress `kind: test` services |
| runner查找不存在的`skiff/language/scripts/skiff.mjs`并传退役`--packages-dir` | current main script不可执行 | prepared runner使用`<SKIFF_ROOT>/scripts/skiff.mjs`、fresh artifact/Cargo roots和current CLI |

这些不是I6 carrier导致的新变化，而是尚未进入`main`的既有26个Phase 05提交。I7不得重新实现。
Preferred I7 input是prepared branch `19cfab5d`；official repo在该input上没有新的I6-driven
production diff，只是consumer gate。若integration owner要求从requested `main`执行，则先严格
fast-forward/merge现有prepared branch，不能发一个重复production实现任务。

`http-session`和`track`只使用current unary std HTTP shapes、config、time/random、DB和package call；
没有timeout、external HTTP stream、file、Actor、WebSocket、service call、receive/requestId/cancel。
它们不能充当I6 carrier receipt。Package repo也没有checked-in artifact golden。

prepared runner的离线preflight已实际通过：

```bash
cd /Users/geek/workspace/skiff-packages-phase-05-integration
SKIFF_ROOT=/Users/geek/workspace/skiff-p5-f445h-i7r-preflight \
  node scripts/test-packages.mjs --list http-session,track
node --check scripts/test-packages.mjs
```

它列出两个非零offline test roots并声明`externalRequests: false`。

## Minimal executable I7 DAG

### DAG和并行波次

```text
final I6 acceptance
  ├─ S0 Skiff real-source/artifact checkpoint ─┬─ S1 Host/Router receipt ───────┐
  │                                            ├─ C  Codex Relay + AIHub ─┐    │
  │                                            └─ A  Agine service/Host ──┼─ U ┤
  ├─ P0 official-package prepared consumer gate ────────────────┘        │    │
  └─ T0 Internals current isolated-gate checkpoint ─────────────┘        │    │
                                                                          │    │
                       S1 + P0 + T0 + C + A + U ───────> J hermetic join ─┘
                                                            |
                                                            v
                                             L0 stable/API/Host validation
                                                            |
                                                            v
                                           L1 browser + separately-authorized live
```

预计I6 acceptance之后有6个串行wave：

1. `S0 || P0 || T0`
2. `S1 || C || A`
3. `U`
4. `J`
5. `L0`
6. `L1`

关键路径是两条等价join path中的较慢者：

```text
I6 -> S0 -> A -> U -> J -> L0 -> L1
I6 -> S0 -> C -> U -> J -> L0 -> L1
```

`S1`和official package gate也在`J`前强制join。任何一个repo的失败只能回到该repo的新leaf，不得由
跨repo coordinator顺手修改。

### S0 — Skiff real-source/artifact checkpoint

- **直接父节点 / blocked-by**：最终I6 acceptance、F442C、F443B、I6S、F444C、F445C；
  当前只被最终I6 acceptance阻塞。
- **Repository / owner**：Skiff；compiler/test-runner fixture owner；独立Skiff worktree。
- **允许写集**：
  - 新建`test-runner/fixtures/package-service-current-scope/**`；
  - `test-runner/tests/package_service_contract_deployment.rs`；
  - `compiler/tests/timeout_artifact_lowering.rs`或一个新的单一current-scope compiler integration test；
  - `test-runner/src/canonical_package/tests/combined.rs`仅用于F76 current test-service provenance刷新。
- **禁止写集**：所有compiler/runtime/Host/Router production；Internals；`skiff-packages`；
  schema/identity constants；stable/live scripts。
- **第一处预期修改**：checked-in split-manifest `.skiff` fixture，覆盖nested timeout下的HTTP
  unary/stream、WebSocket outbound request、file、Actor和first service call。
- **真实RED**：当前tracked `.skiff` timeout零匹配；不存在要求的fixture/test target。
- **Positive**：同一source编译出exact File IR、PackageArtifact、ServiceContract、
  DeploymentArtifact、GatewayEntry和RuntimeAssembly identities。
- **Negative**：inline ingress、旧identity、timeout fact mutation、错误package/interface owner和
  旧call syntax fail closed；F76按current `tests/*` roots取得非零case。
- **Focused commands**：

  ```bash
  cargo test -p skiff-compiler --test timeout_artifact_lowering
  cargo test -p skiff-test-runner --test package_service_contract_deployment \
    canonical_live_source_roots_compile_to_current_receipts
  P5_F76_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
    cargo test -p skiff-test-runner p5_f76_contextual_callable_provenance_combined -- --ignored
  ```

- **完整命令和唯一owner**：S0 owner只跑`node scripts/verify.mjs --only compiler,test-runner`；
  全Skiff gate留给`J`，不重复。
- **并行**：可与`P0`、`T0`并行；同一Skiff integration上不得同时有另一个fixture owner。
- **Commit/worktree/integration**：test/fixture commit与result commit分开；验收后由Skiff integration
  owner合并；不push。
- **证据失效**：I6 production、compiler lowering/schema/identity、official test-root layout或fixture
  source任一变化都会使receipt失效。

### P0 — official-package prepared consumer gate

- **直接父节点 / blocked-by**：最终I6 acceptance；prepared package commit `19cfab5d`；
  当前只等待I6 acceptance。
- **Repository / owner**：`skiff-packages`；preferred input是现有Phase 05 worktree。
- **允许写集**：preferred route为空，是read-only consumer gate。若必须从`main`开始，只允许
  integration owner合并现有prepared branch；不允许重新编码相同修复。
- **禁止写集**：所有fresh production/test修改、Registry扩展、Skiff/Internals文件、live OpenAI test。
- **第一处预期动作**：离线`--list`，随后只运行reachable consumer gate；不是空implementation。
- **真实RED**：requested `main`的public API/state/slash-call/test-runner首错；prepared branch必须
  在gate前被精确pin。
- **Positive**：`http-session`先发布，`track`后发布；两个零-ingress test service非零执行。
- **Negative**：错误package ref、旧dot callable、无state requirement、legacy sidecar fail closed。
- **Focused/full commands**：

  ```bash
  cd /Users/geek/workspace/skiff-packages-phase-05-integration
  SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
    node scripts/test-packages.mjs --list http-session,track
  SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
    node scripts/test-packages.mjs http-session,track

  # 只由package integration owner执行一次；不属于I6 critical receipt
  SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
    node scripts/test-packages.mjs --all
  ```

- **唯一owner / isolation**：runner创建fresh artifact和Cargo roots，使用managed isolated test
  stack；需要本机`cargo`、`node`、`mongod`，不碰stable/OAuth/browser/network。
- **并行**：与`S0`、`T0`并行；Agine `A`的final compile依赖本gate。
- **Commit/worktree/integration**：read-only route无implementation commit；若promotion需要，把现有
  branch按repo integration流程合并到`main`，不生成重复leaf、不push。
- **证据失效**：final Skiff compiler/std/identity、package branch或test-service roots变化。

### T0 — Internals current isolated-gate checkpoint

- **直接父节点 / blocked-by**：最终I6 acceptance和current Skiff CLI；当前只等待I6 acceptance。
- **Repository / owner**：Internals；唯一shared test-tooling owner；base `19d41001`。
- **允许写集**：
  - `scripts/isolated-service-graph.mjs`；
  - `scripts/check-isolated-service-graph.mjs`；
  - `scripts/test-isolated-service.mjs`；
  - `scripts/isolated-service-graph.test.mjs`；
  - `scripts/test-isolated-service.test.mjs`。
- **禁止写集**：任何service/client/Host production或test fixture；Skiff/packages；stable package store。
- **第一处预期修改**：把旧`--config`/`--packages-dir`/`--service-artifact-root` test invocation迁到
  required `--artifact-root`和current inline-effects/managed isolated runtime。
- **真实RED**：current wrapper对fixed Skiff传已明确拒绝的CLI选项；旧路径还默认共享27017。
- **Positive**：fake-spawn unit精确断言三service拓扑、shared temp artifact/build root和current
  args；non-live test不读取`.skiff-instance`/reload URL。
- **Negative**：旧flags、stable paths、shared Mongo URL、零test selection fail closed。
- **Focused commands**：

  ```bash
  cd /Users/geek/workspace/internals
  node --test scripts/isolated-service-graph.test.mjs scripts/test-isolated-service.test.mjs
  node --check scripts/isolated-service-graph.mjs
  node --check scripts/test-isolated-service.mjs
  ```

- **完整命令和唯一owner**：真实service matrix不在T0运行，由`J`在各service已迁移后运行一次。
- **并行**：与`S0`、`P0`并行；`C`和`A`共用其输出，禁止各自再改shared scripts。
- **Commit/worktree/integration**：独立Internals worktree和test-tooling commit；合并到Internals
  Phase 05 integration后再扇出；不push。
- **证据失效**：Skiff CLI/test-runner isolation contract或Internals service graph变化。

### S1 — Host/runtime/Router cross-layer receipt

- **直接父节点 / blocked-by**：`S0`和最终I6 acceptance。
- **Repository / owner**：Skiff；Host/Router integration-test owner。
- **允许写集**：
  - S0 fixture的test harness；
  - `runtime/host/src/host/router_session/tests/**`中精确新增/更新的integration tests；
  - `router/tests/helpers/compilerArtifacts.ts`；
  - `router/tests/compilerGeneratedManifestCompatibility.test.ts`；
  - `router/tests/assembly-http-gateway-stream.test.ts`；
  - `router/tests/runtime-assembly-unary-dispatch.test.ts`；
  - `router/tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts`；
  - `router/tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts`；
  - F442 cross-system verifier/corpus只在exact identity事实变化时机械刷新。
- **禁止写集**：runtime/Host/Router production、protocol/schema、Internals/packages、stable/live。
  若真实receipt暴露production bug，本节点停止并新建repo-local leaf。
- **第一处预期修改**：让Host和Router tests消费S0 exact compiled artifact，而不是synthetic snapshot
  或Rust-generated source。
- **真实RED**：当前不存在连续source → artifact → Host → Router test target。
- **Positive**：
  - HTTP unary和server-stream的status、headers、chunk order、single end；
  - WebSocket outbound RPC三参数/hidden id；
  - nested timeout下五类carrier的deadline、tie-break和late-settlement；
  - first service call不读deployment timeout。
- **Negative**：old identities、receive branch、peer request id、public cancel/`-32800`、
  legacy service relay、late response和wrong generation均fail closed。
- **Focused commands**：

  ```bash
  cargo test -p skiff-runtime-host \
    host_http_gateway_typed_raw_and_stream_execute_private_handlers
  pnpm --dir router exec vitest run \
    tests/compilerGeneratedManifestCompatibility.test.ts \
    tests/assembly-http-gateway-stream.test.ts \
    tests/runtime-assembly-unary-dispatch.test.ts \
    tests/runtime-assembly-websocket-jsonrpc-protocol.test.ts \
    tests/runtime-assembly-websocket-jsonrpc-dispatch.test.ts
  node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
  node cross-system-fixtures/package-service-ecosystem/verify.mjs --combined-probe
  node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
  ```

- **完整命令和唯一owner**：`node scripts/verify.mjs --only runtime,router,test-runner`；
  全仓`verify`只在`J`运行一次。
- **并行**：与`C`、`A`并行；是唯一Skiff test owner。
- **Commit/worktree/integration**：独立Skiff worktree；tests与result分开commit；经Skiff integration
  owner合并，不push。
- **证据失效**：S0 artifact、I6 Host/native、Router transport/identity或F442 corpus变化。

### C — Codex Relay + bounded AIHub caller

- **直接父节点 / blocked-by**：`S0`、`T0`、最终I6 acceptance；不依赖Agine implementation。
- **Repository / owner**：Internals；同一repo内Codex Relay provider和唯一AIHub caller owner。
- **允许写集**：
  - `codex-relay/service/service.yml`及新`package.yml`/`http.yml`；
  - `codex-relay/service/api.yml`只在exact compiler RED证明需要时；
  - `relay.skiff`、`proxy_runtime.skiff`、`responses_projection.skiff`；
  - `relay_responses_projection.test.skiff`、`package_response_health.test.skiff`、
    `relay_routes.test.skiff`及该service的current inline test config；
  - 删除/迁移`codex-relay/service/skiff.test-doubles.json`；
  - `aihub/service/service.yml`及其split manifest；
  - `aihub/service/internal/aihub_service.skiff`和`aihub_service.test.skiff`；
  - 删除/迁移`aihub/service/skiff.test-doubles.json`。
- **禁止写集**：Agine、shared Internals scripts、llm package public APIs、Skiff/packages、
  OAuth/admin runtime、external login。
- **第一处预期修改**：split Codex Relay/AIHub authoring；新增成功API-key server-stream
  hermetic fixture。
- **真实RED**：legacy inline manifest先失败；成功external stream没有start→chunks→end receipt。
- **Positive**：
  - Relay tagged unary export；
  - AIHub exact `codexRelay/relayProxy.responsesCompletedResult`call；
  - status、filtered headers、surviving ordered chunks/events、single end；
  - source/artifact使用current service identity。
- **Negative**：failure-before-start、failure-after-start、malformed stream、old service relay、
  deployment timeout reuse、public cancel、unfiltered sensitive headers。
- **Focused commands**：

  ```bash
  cd /Users/geek/workspace/internals
  node scripts/check-isolated-service-graph.mjs agine.ai/codex-relay
  node scripts/test-isolated-service.mjs agine.ai/codex-relay
  node scripts/check-isolated-service-graph.mjs agine.ai/aihub
  node scripts/test-isolated-service.mjs agine.ai/aihub
  ```

- **完整命令和唯一owner**：Codex Relay → AIHub graph的完整non-live source/test由C owner运行一次；
  real OAuth/network不在该命令。
- **并行**：与`S1`、`A`并行；`U`和`J`等待C。
- **Commit/worktree/integration**：一个Internals worktree；provider commit在caller commit之前；
  合并到Internals Phase integration；不push。
- **证据失效**：Relay/AIHub API identity、I6 service-call carrier、llm package stream shape或shared
  test runner变化。

### A — Agine service + protocol + Host terminal cutover

- **直接父节点 / blocked-by**：`S0`、`P0`、`T0`、最终I6 acceptance；使用F445C和F444C stash
  provenance。
- **Repository / owner**：Internals；Agine service/Host boundary owner；base `19d41001`，在新clean
  worktree中materialize stash，不修改原stash。
- **允许写集**：
  - `agine/service/service.yml`及新`package.yml`/`http.yml`/`websocket.yml`；
  - `agine/service/api.yml`；
  - `internal/agine_service.skiff`、`agine_http_routes.skiff`；
  - 新`host_peer_rpc.skiff`、`host_peer_protocol.skiff`及相关`.test.skiff`；
  - 删除/迁移current DB-relay files和`agine/service/skiff.test-doubles.json`；
  - `agine/protocol/hostPeer.ts`、fixture和`test/hostPeer.*`；
  - `agine/host/src/hostPeerFrame.ts`、`hostPeerHost.ts`、
    `GatewayClient.ts`、`HostService.ts`及其focused tests。
- **禁止写集**：Agine client、Codex Relay/AIHub、shared Internals scripts、Skiff/packages、
  stable Host home/process、service timeout复用、receive/peer methods。
- **第一处预期修改**：在新worktree应用terminal stash，立即以final Skiff重跑F444C exact consumer
  compile；首错必须不再是interface identity。
- **真实RED**：requested main的`unknown field http`；phase candidate仍有receive/inline authoring；
  consumer exact GREEN不存在。
- **Positive**：
  - separate manifests、43 typed/raw HTTP entries、connect-only WebSocket；
  - `host.files.list`、`host.files.search`、`host.current-directory`三参数outbound RPC；
  - business params/results无`id`/`requestId`；
  - Host fake peer执行list/search/current-directory并保留deadline/error。
- **Negative**：recursive JSON/TS transport-id rejection、receive/public peer method、四参数request、
  legacy DB relay、deployment timeout/callee timeout、wrong interface owner fail closed。
- **Focused commands**：

  ```bash
  cd /Users/geek/workspace/internals/agine
  npm run type-check:service
  npm run type-check --workspace @agine/protocol
  npm test --workspace @agine/protocol
  npm run type-check:host
  npm test --workspace @agine/host
  ```

- **完整命令和唯一owner**：Agine service real `.test.skiff` matrix由A owner在T0 runner上运行；
  full Internals join留给`J`。
- **并行**：与`S1`、`C`并行；其compile可使用pinned pre-C public API，但C变化后由`J`重跑完整
  graph；`U`等待A和C。
- **Commit/worktree/integration**：service、protocol/Host、result分别commit；验收后合并Internals
  Phase integration；不pop stash、不push。
- **证据失效**：F445C/interface identity、S0 artifact、official package builds、Host protocol fixture、
  I6 timeout carrier或C public API变化。

### U — Agine client HTTP-up / WebSocket-down follower

- **直接父节点 / blocked-by**：`A`和`C`。
- **Repository / owner**：Internals；Agine client和API smoke source owner。
- **允许写集**：
  - `agine/client/src/lib/http.ts`、`ws.ts`；
  - `hostFileApi.ts`、`toolproviderApi.ts`、`threadHostBindings.ts`及focused tests；
  - `stores/appStore/messageActions.ts`及focused test；
  - `agine/client/e2e/api.chat-smoke.mjs`及其support lifecycle/cleanup tests；
  - 删除不再使用的`cookie-websocket-rpc` helper只在零caller证明后允许。
- **禁止写集**：service/Host/protocol、Codex Relay/AIHub、shared scripts、Skiff/packages、browser
  profile、stable/live state。
- **第一处预期修改**：删除generic WebSocket business request/send surface；把smoke的agent create和
  cleanup迁HTTP。
- **真实RED**：current `ws.ts`公开request/send，多个production callers和smoke仍发送WS uplink。
- **Positive**：所有business mutation走HTTP；WebSocket只connect/observe downlink；
  unit测试断言HTTP request和downlink event。
- **Negative**：client public type中无generic request/send；source search无business WS uplink；
  smoke不再构造transport requestId。
- **Focused/full commands**：

  ```bash
  cd /Users/geek/workspace/internals/agine
  npm run type-check:client
  npm run test:logic --workspace @agine/client
  ```

  API smoke和Playwright不在U执行。

- **并行**：A/C join后执行；不能与另一个client owner并行。
- **Commit/worktree/integration**：独立Internals client worktree/commit；合并Phase integration；
  不push。
- **证据失效**：Agine HTTP/WS contract、Codex/AIHub chat path、smoke API或client protocol变化。

### J — final hermetic cross-repository join

- **直接父节点 / blocked-by**：`S1`、`P0`、`T0`、`C`、`A`、`U`的精确commits/trees。
- **Repository / owner**：read-only gate coordinator；不拥有任何implementation。
- **允许写集**：空。
- **禁止写集**：三个repo全部production/tests、stable、Mongo singleton、OAuth/browser/network。
- **第一处动作**：记录三repo exact HEAD/tree/dirty和dependency revisions；dirty即停止。
- **RED/positive/negative**：必须先看到各leaf的真实RED记录；J只接受所有focused positive/negative
  已存在且final-tree rerun GREEN，不以历史count替代。
- **命令和唯一owner**：

  ```bash
  # Skiff：final non-live gate，仅一次
  cd /Users/geek/workspace/skiff-phase-05-integration
  node scripts/verify.mjs

  # Official reachable closure
  cd /Users/geek/workspace/skiff-packages-phase-05-integration
  SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
    node scripts/test-packages.mjs http-session,track

  # Internals source/runtime/client/Host
  cd /Users/geek/workspace/internals/agine
  npm run type-check:service
  npm test --workspace @agine/service
  npm run type-check:client
  npm run test:logic --workspace @agine/client
  npm run type-check:host
  npm test --workspace @agine/host
  npm run type-check --workspace @agine/protocol
  npm test --workspace @agine/protocol
  ```

- **并行**：各repo focused gates可并行，但每个Cargo target和artifact root必须唯一；每repo complete
  gate只有一个owner。最终join在全部完成后。
- **Commit/worktree/integration**：无commit；失败返回精确repo owner的新leaf；不跨repo修。
- **证据失效**：任一repo HEAD/tree、package store、generated artifact或test dependency变化。

### L0 — stable, API chat and Host validation

- **直接父节点 / blocked-by**：`J`，以及三个repo分别完成本地integration/merge后的exact candidate。
- **Repository / owner**：无implementation；一个shared-state/live owner。
- **允许写集**：空；只在用户明确授权后允许构建runtime binary、restart component、更新watch registry
  （若确实缺项）、reload final artifact cohort和restart所选Host process。
- **禁止写集**：source/tests、real OAuth、browser profile、未授权external provider mutation。
- **第一处动作**：读回stable health、watch registry和loaded exact service/version；不先跑chat。
- **Positive**：按Codex Relay → AIHub → Agine顺序构建/reload同一cohort；session/list、connect-only
  WS、HTTP create/send/get、downlink delta、Host三method probe和local fake raw-stream probe。
- **Negative**：mixed artifacts、wrong generation、old receive/uplink、missing Host method、provider error
  必须由最小诊断暴露，而不是只报chat timeout。
- **前置命令**：

  ```bash
  cd /Users/geek/workspace/skiff
  node scripts/build-dev-runtime.mjs
  node scripts/skiff.mjs instance restart .skiff-instance/config.yml runtime

  # final artifacts已按dependency cohort加载、Host已按授权restart后
  cd /Users/geek/workspace/internals/agine
  npm run e2e:chat-smoke
  ```

- **唯一owner / shared state**：stable ports 4000/4001/4002/4003、Mongo 27017、global watch registry、
  Host process/home和provider config由同一owner串行操作；不能与其它stable任务并行。
- **Commit/integration**：无commit；该gate之前各repo必须各自完成本地integration，不能从三个临时
  worktree拼mixed state；不push。
- **证据失效**：runtime binary、artifact cohort、registry、Mongo state、Host process或provider
  config变化。

### L1 — browser and separately authorized live gates

- **直接父节点 / blocked-by**：`L0` API/Host GREEN。
- **Repository / owner**：read-only shared browser/live owner。
- **允许写集**：空；只允许用户明确授权的browser session、real OAuth或external upstream请求。
- **禁止写集**：source/tests、自动化real OpenAI login、并行使用localhost OAuth callback 1455。
- **第一处动作**：先用既有Chrome执行browser smoke；OAuth/external upstream/runtime-live分开授权、
  分开记录。
- **Positive**：browser UI完整chat；如获授权，real provider和explicit runtime-live generation。
- **Negative**：browser失败必须保留API smoke GREEN作为分层证据；real OAuth失败不得反推
  source/runtime失败。
- **命令**：

  ```bash
  cd /Users/geek/workspace/internals/agine/client
  npm run test:frontend

  # 仅在五个exact输入和用户授权均具备时
  cd /Users/geek/workspace/skiff
  node scripts/verify.mjs --only runtime-live \
    --runtime-live-activation-url <exact-url> \
    --runtime-live-ingress-url <exact-url> \
    --runtime-live-artifact-root <exact-root> \
    --runtime-live-environment <exact-id> \
    --runtime-live-expected-generation <exact-n>
  ```

- **唯一owner / commit / invalidation**：browser、OAuth callback和external live各自单例owner；无commit。
  browser profile、account session、provider config、generation或artifact任一变化都会使live evidence
  失效。

## Gate preflight and authorization boundary

### 已确认的selector/dry-run

实际执行：

```bash
node scripts/verify.mjs --only compiler,runtime,router,test-runner --list
```

返回6个非零phase：compiler Rust、runtime artifact-boundary self-test、runtime artifact-boundary、
runtime Rust、Router和test-runner Rust。完整`node scripts/verify.mjs --list`返回279个non-live
phase。

实际执行：

```bash
node scripts/verify.mjs --only runtime-live --list
```

只展开一个blocked phase，并精确要求activation URL、ingress URL、artifact root、environment和
expected generation。没有这些输入时不会误跑live。

### Dependency和隔离事实

- 本任务Skiff worktree没有`router/node_modules`，所以只可做selector listing；Skiff Phase 05
  integration checkout已有lockfile-compatible Router dependencies。未来worktree任务必须显式使用
  已批准的依赖provisioning；本预检没有安装或建立链接。
- `cargo`、`node`、`pnpm`、`mongod`、`mongosh`当前可发现。
- Internals requested main的Agine workspace已有dependencies；Phase 05 worktree只有部分client
  dependencies，Host依赖不完整。最终integration gate应在依赖完整的final checkout运行；leaf不得
  悄悄安装后把安装状态当receipt。
- package runner不依赖repo `node_modules`，自己创建fresh artifact root和`CARGO_TARGET_DIR`。
- 并行Rust tasks必须使用不同`CARGO_TARGET_DIR`；并行service/package tasks必须使用不同artifact、
  build和Mongo/port lease。不得共享临时store后声称hermetic。
- Internals type-check会准备local package store，因此虽然不访问stable，它也不是“零filesystem
  mutation”；唯一owner必须记录source roots和清晰的temp/store边界。

### Hermetic first, shared state last

可以在final I6后提前运行：

- compiler/test-runner/Host/Router focused tests；
- official package isolated consumer tests；
- Internals source compile、Node/TypeScript/Skiff non-live tests；
- client logic、Host fake/loopback和protocol tests；
- Codex stream fake upstream receipt。

必须留到final integrated candidate：

- Skiff完整non-live `verify`；
- exact cross-repo source graph；
- stable runtime binary rebuild/restart；
- service artifact cohort reload；
- API chat smoke和真实Host process；
- browser；
- real OAuth、external provider和runtime-live。

需要用户明确授权和唯一owner的操作：

1. stable instance、global watch registry、27017 Mongo、runtime restart和artifact reload；
2. 真实Host process/home restart；
3. Chrome/browser profile；
4. real account/OAuth callback 1455；
5. external provider/network和runtime-live target。

这些是后续执行授权，不是当前架构决策。

## Historical evidence invalidation and masking

### F442

F442 implementation commit `fa3a8ed...`拥有的四个corpus/verifier路径从该commit到固定Skiff input
字节不变，所以三个verifier命令仍是有效的test入口。但：

- 后续Router JSON-RPC production/tests已变化；
- F442记录的历史Router 164和Rust 19 exact counts已过期；
- F442 corpus没有I6 current-scope carrier；
- 如果final I6改变transport/identity，wire evidence必须全部刷新。

### F443

F443历史证据不能复用为final acceptance：

- Gate A锚定旧Skiff `735bf1c4...`，此后compiler/runtime/Router已大量变化；
- Gate B是external Registry receipt，不是current runtime carrier evidence；
- Internals F443 anchor到current main已有大量Agine/AIHub/Codex Relay变化；
- package anchor到current prepared branch的manifest/test-service layout已变化；
- 旧Registry receipt命令和test-doubles路径不再是current canonical gate。

### I6和分层遮挡

- final I6 Host/native/eval production会使所有pre-I6 HTTP、WebSocket request、file、Actor、time、
  response-stream behavior receipt失效。
- compiler/artifact receipt只有在I6不改变lowered facts/schema/identity时才可沿用。
- source/config失败会遮挡compiler；compiler/identity失败会遮挡loader；loader失败会遮挡Host；
  Host失败会遮挡Router；Router失败会遮挡client。
- Router socket mock通过不能证明source/compiler；Host generated fixture通过不能证明Router；
  compiler artifact通过不能证明capability behavior。
- synthetic old-identity snapshot会绕过current artifact reader。
- chat smoke失败可能来自artifact cohort、session/Mongo、provider、Host或client；必须先按最小诊断
  顺序分层。
- chat smoke当前不含Host leg，也不含Codex external stream；不能用一个GREEN替代三个receipt。
- 任一final gate若使用不同repo commits、mixed loaded artifacts或被修改的shared state，整个
  downstream evidence无效。

## Ready queue and user decisions

当前ready queue：

```text
无。S0、P0、T0都只等待最终I6 acceptance。
```

最终I6 acceptance到达后，可立即并行启动且无需新增设计回答的节点：

```text
S0 Skiff real-source/artifact checkpoint
P0 official-package prepared consumer gate
T0 Internals current isolated-gate checkpoint
```

当前需要用户回答的架构/产品问题：**无**。

已有provenance足以选择prepared package branch、prepared Internals branch和F444C stash；不需要让
用户在“旧main上重做”与“复用prepared input”之间选择。后续到达`L0`/`L1`时，执行Agent必须分别
请求stable/Mongo/Host、browser、OAuth/external live授权；未获授权时只停止相应live gate，不得用
live缺失否定已经完成的hermetic receipt。
