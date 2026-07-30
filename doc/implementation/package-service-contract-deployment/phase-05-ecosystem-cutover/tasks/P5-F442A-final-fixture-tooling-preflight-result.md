# P5-F442A Final fixture/tooling closeout preflight result

状态：`PREFLIGHT_COMPLETE / DAG_READY / CHEAP_COMBINED_BLOCKED`。

本只读 leaf 已把 combined 前的剩余清理收敛为三个可并行的 blocking owner：

1. Rust/package-test/Host/test-runner fixture；
2. cross-system corpus/verifier；
3. 默认 source-layout checker。

Router/Runtime README 是第四个写集互斥、可并行但不阻塞 cheap combined 的文档节点。没有发现
新的公共契约或可达 production owner，因此不返回 `TASK_SCOPE_EXPANDED`。
`runtime/host/src/host/register_mapper.rs` 虽位于 `src/`，但只由
`runtime/host/src/host/mod.rs` 的 `#[cfg(test)] mod register_mapper;` 编译；全仓没有它自己的
测试以外的 consumer。它是应删除的旧 test-only mapper，不是 production owner。

## 1. 基线与只读范围

| 项目 | 值 |
| --- | --- |
| worktree | `/Users/geek/workspace/skiff-p5-f442a-fixture-tooling-preflight` |
| branch | `codex/p5-f442a-fixture-tooling-preflight` |
| task start HEAD | `542a71b1183fe98eb2ff41ac4d5c8872954288d0` |
| current identities | GatewayEntry v2；ServiceProtocol v5；DeploymentArtifact v3；RuntimeAssembly v2 |

只读取了 leaf 指定的 fixture、runtime/package-test、runtime/host、test-runner、checker、
两份 README、current std/prelude/compiler builtin owner及直接 consumer；设计输入只使用了四个
直接父节点。没有启动 stable、watch、MongoDB、server、live 或外部网络。

## 2. 失败/过期矩阵

| Surface | Current owner/shape | Stale path | Earliest direct failure | Classification |
| --- | --- | --- | --- | --- |
| package-test package dependency fixture | `PackageRequirement`与`PackageBinding`都携带`collection_name_mapping`；无映射时为空 map | `runtime/package-test/tests/support/mod.rs:732,809` | `E0063`，目标测试尚未编译 | `BLOCKING_FIXTURE` |
| Host assembly activation fixture | `AssemblyIdentity`只接受`skiff-runtime-assembly-v2` | `runtime/host/src/host/router_session/tests.rs:651`仍为v1 | `cargo test -p skiff-runtime-host --lib`中4个activation测试在fixture decode处失败 | `BLOCKING_FIXTURE` |
| test-runner WebSocket source golden | compiler生成的PackageBuild及其派生identity tuple | `test-runner/tests/package_service_contract_deployment.rs:2768-2807`中的首个旧build golden | 首个actual为`87120182…50ad`，expected为`5ce08903…25880` | `BLOCKING_FIXTURE` |
| shared generic request fixture | generic legacy `request.start` branch仍存在，但其ServiceProtocol必须是v5 | `runtime-request-wire.json`的`legacyRequestStartHeaders[0]`仍为v3 | production outer validator先报“must be skiff-service-protocol-v5”，未到达“legacy-only”断言 | `BLOCKING_FIXTURE` |
| external authoring checkpoint | `service.yml`只拥有`id`/`serviceCalls`，test service另有`kind: test`；HTTP/WebSocket分别由`http.yml`/`websocket.yml`拥有；timeout在profile config | `checkpoint.json`仍声明`service.yml = id,http,websocket,timeout` | verifier self-test把过期checkpoint自证为正确，不能发现external manifest回归 | `BLOCKING_FIXTURE` |
| checkpoint assertions | verifier应断言split external manifests，而不是旧inline fields | `verify.mjs`直接断言旧`service.yml`字段 | semantic false-green；与当前compiler owner不一致 | `BLOCKING_TOOL` |
| obsolete WebSocket response corpus | current connect response在`runtime-websocket-connect-wire.json`；JSON-RPC response由current bridge/profile tests拥有 | 无consumer的`runtime-websocket-response-wire.json`含positive `webSocketReceive`、ServiceProtocol v2、`skiff-gateway-v1` | 无直接consumer；全仓consumer搜索为0 | `NON_BLOCKING_FOLLOW_UP` |
| old receive/route mapper double | current Host通过typed RuntimeAssembly route与`websocket.jsonRpcParams`执行 | `#[cfg(test)]`的`runtime/host/src/host/register_mapper.rs`构造`receive`树、Gateway v1和ServiceProtocol v2 | Host lib中7个自包含mapper测试会通过，因而是假正例而非production验证 | `NON_BLOCKING_FOLLOW_UP` |
| incidental opaque identity fixtures | current positives应使用RuntimeAssembly v2、ServiceProtocol v5、DeploymentArtifact v3 | Host actor/outbound test helpers仍有assembly v1/protocol v2；test-runner orchestration helper仍有deployment v2 | targeted orchestration test仍通过，证明这些unchecked/opaque值尚未触发validator | `NON_BLOCKING_FOLLOW_UP` |
| stale-generation negatives | current parser必须拒绝RuntimeAssembly v1、GatewayEntry v1及旧`skiff-gateway-v1` | activation raw case、runtime request mutations、connect routing/metadata mutations | 正确命中reject path | `DELIBERATE_NEGATIVE` |
| legacy receive rejection probes | connect header必须拒绝`receiveEvent`和`websocketAdapter:{kind:"receive"}` | `runtime-websocket-connect-wire.json`中两条named mutation | 正确命中reject path；不得与positive receive fixture一起删除 | `DELIBERATE_NEGATIVE` |
| runtime frame/generation ids | `request.start`、response、request.cancel及generation acquire/release使用内部transport correlation | `runtime-request-wire.json`与`websocket-generation-lifecycle-wire.json`中的`requestId` | current Router/Runtime consumer按id配对 | `CURRENT_INTERNAL_TERM` |
| JSON-RPC ids | peer id只在profile/broker中；Runtime得到独立frame `requestId`；handler只得到typed params及可选connection/business identity | 无current业务DTO暴露；旧README声称业务frame/response有`requestId?` | current bridge tests证明同值peer id与runtime id使用不同namespace | `CURRENT_INTERNAL_TERM` |
| cancellation | Host `RuntimeError::Cancelled`、CancellationToken、`request.cancel`与`$/cancelRequest`是不可捕获terminal/control；`std.service.InternalError`仍是current public error | `check-skiff-source-layout.mjs`仍要求已删除public builtin `CancelError` | checker立即输出`FAIL compiler builtin registry must own CancelError` | `BLOCKING_TOOL` |
| builtin/std surface checker | compiler registry拥有`Actor`，无`ActorRef`/`CancelError`；`std/api.yml`拥有current HTTP/file/WebSocket/service inventory | checker漏掉`Actor`、`InternalError`、WebSocket request/types及多数current HTTP helpers；file inventory当前完整 | 删除错误CancelError要求后，现有checker仍会对这些删除产生false-green | `BLOCKING_TOOL` |
| Router README | external manifests、双向declared JSON-RPC、observed writer、current identities | receive/route/automatic response、v3 protocol、旧artifact/publish topology | 不被默认checker或聚焦tests消费 | `STALE_DOCUMENTATION` |
| Runtime README | current `send*ToConnection`/business identity、`requestJsonToConnection`、无raw receive、current identities | v2 protocol、旧`sendText/sendBinary/sendJson`、receive handler及旧artifact layout | 不被默认checker或聚焦tests消费 | `STALE_DOCUMENTATION` |
| `publication`内部术语 | `manifest.publication`、`PublicationSourceGraph`、`read_publication_resources`仍拥有compiler source/resource输入；Host commit/stream publication也是内部状态术语 | README中无额外current信息的旧published build/service-assembly段落 | 代码consumer仍真实使用内部术语；不能机械全文改名 | `CURRENT_INTERNAL_TERM` |
| test service/profile/live cleanup | `kind:test`固定`config.skiff-test.yml`；target environment只属于live CLI；inline effects投影为内部test doubles | 指定obsolete live case/helper反向搜索均为0；tracked fixtures均为split manifest | current focused tests通过相应profile/source-root断言 | `CURRENT_INTERNAL_TERM` |

### 2.1 不得误删的 negative

以下旧字符串是明确的reject corpus，应保留：

- `activation-raw-cases.json`的`runtime assembly v1 identity`；
- `runtime-request-wire.json`的assembly v1、GatewayEntry v1及旧gateway prefix mutations/raw case；
- `runtime-websocket-connect-wire.json`的GatewayEntry v1 routing/metadata、旧gateway prefix、
  `receiveEvent`和`websocketAdapter` mutations；
- generation lifecycle corpus中的wrong frame、request-cancel substitution、duplicate id等negative。

generic `legacyRequestStartHeaders`也不能整段删除：Router schema仍有独立generic legacy branch，
Router与Rust transport consumer都明确验证它。这里只把其中的ServiceProtocol v3刷新为v5，使
outer transport合法后继续证明它不是RuntimeAssembly typed request。

## 3. requestId / receive / cancellation分类

### 3.1 必须保留的transport correlation id

- Runtime frame `request.start`/response/`request.cancel`的`requestId`；
- Runtime WebSocket JSON-RPC dispatch frame的独立`requestId`；
- WebSocket generation lifecycle acquire/release的
  `skiff-websocket-lifecycle-request-v1:opaque:*`；
- control-plane、actor和telemetry中各自有真实pending owner的内部correlation。

它们用于pending map、response demux、cancel和generation fencing，不是业务字段。

### 3.2 必须隐藏于业务层的JSON-RPC id

- peer-originated JSON-RPC id允许非空string或safe integer，只由profile/broker保存并原类型回显；
- platform-originated peer id只由connection-generation内的broker生成；
- Router为peer request另建Runtime frame `requestId`，Host handler只绑定完整
  `websocket.jsonRpcParams`，可另取`websocket.connectionId`和
  `websocket.businessIdentity`；
- peer id与Runtime frame id即使文本相同也属于不同namespace，不得进入业务DTO。

### 3.3 必须删除的旧业务DTO/route receive id

- `router/README.md`中的`{ path, requestId?, ok, payload, error? }`自动响应说明；
- README中的`ConnectionMessage`、route frame、receive fallback；
- 无consumer的`runtime-websocket-response-wire.json`整份positive receive corpus。

该旧corpus的response header `requestId`本身是transport id；删除原因是整份receive phase和旧
identity owner已失效，不是因为transport id应消失。

### 3.4 obsolete receive/route/automatic response fixture

- 删除positive/orphan `runtime-websocket-response-wire.json`；
- 删除test-only `register_mapper`及`host/mod.rs`中的cfg-test module声明；
- 保留connect corpus中的named receive rejection mutations；
- current `runtime-websocket-connect-wire.json` connect response与current JSON-RPC bridge tests
  继续作为owner。

### 3.5 cancellation

必须保留：

- Rust `CancellationToken`/`CancellationSource`、stream cancel signal；
- Runtime内部`RuntimeError::Cancelled`且无ordinary/wire projection；
- Runtime `request.cancel` control与Router JSON-RPC `$/cancelRequest`；
- first-terminal-wins、late completion fencing、generation/session cleanup。

必须删除的只有checker对public builtin `CancelError`的要求。当前public ordinary fallback是
`std.service.InternalError`；它与不可捕获的cancel terminal不是同一表面。

## 4. 最小实现DAG

### C0：只读canonical checkpoint

无写集。三个blocking节点共同以以下既有owner为输入：

- `artifact-identity/src/constants.rs`：
  ServiceProtocol v5、DeploymentArtifact v3、GatewayEntry v2；
- RuntimeAssembly v2的artifact-model parser；
- `compiler/core/src/prelude_registry.rs`；
- `std/api.yml`与对应`std/*.skiff`；
- split `service.yml`/`http.yml`/`websocket.yml` compiler input owner。

不得让fixture、checker或README各自发明另一份identity/surface常量。

### R：Rust/test-runner fixture owner

依赖：C0。

精确写集：

- `runtime/package-test/tests/support/mod.rs`；
- `runtime/host/src/host/router_session/tests.rs`；
- `runtime/host/src/eval_capability_adapter/actor.rs`；
- `runtime/host/src/eval_capability_adapter/request_contexts.rs`；
- `runtime/host/src/capability_context/actor/tests.rs`；
- `runtime/host/src/capability_context/outbound_service.rs`；
- `runtime/host/tests/active_runtime_assembly.rs`；
- `runtime/host/src/host/mod.rs`；
- 删除`runtime/host/src/host/register_mapper.rs`；
- `test-runner/src/runtime_execution/tests/orchestration.rs`；
- `test-runner/tests/package_service_contract_deployment.rs`。

实现要求：

- 两个missing mapping都用语义正确的empty mapping补齐；
- Host current-positive assembly/protocol fixture刷新到v2/v5；
- 删除无production consumer的test-only旧register mapper；
- orchestration deployment fixture刷新到v3；
- 重新生成/核对四个WebSocket source fixture的完整build/ABI/deployment/assembly tuple，不能只把
  首个expected build替换为本次actual。

聚焦命令：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-package-test --test package_artifact \
  entrypoint_validation_rejects_non_exact_gateway_facts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment
```

### X：cross-system corpus/verifier owner

依赖：C0。可与R、K、D并行。

精确写集：

- `cross-system-fixtures/package-service-ecosystem/checkpoint.json`；
- `cross-system-fixtures/package-service-ecosystem/runtime-request-wire.json`；
- `cross-system-fixtures/package-service-ecosystem/verify.mjs`；
- 删除
  `cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json`。

不需要修改Router或Rust direct consumer：legacy generic branch仍保留，v3→v5不改变corpus shape；
obsolete response file当前没有consumer。

聚焦命令：

```bash
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
node cross-system-fixtures/package-service-ecosystem/verify.mjs --combined-probe
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
router/node_modules/.bin/vitest run --root router \
  tests/protocol.test.ts \
  tests/runtime-assembly-request-wire.test.ts \
  tests/runtime-protocol-websocket-response.test.ts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-transport runtime_assembly_request
```

### K：source-layout checker owner

依赖：C0。可与R、X、D并行。

精确写集：

- `scripts/check-skiff-source-layout.mjs`。

实现要求：

- builtin inventory要求`Actor`，继续拒绝`ActorRef`，删除`CancelError`正向要求并明确拒绝它；
- 以`std/api.yml`/对应source为current owner，补齐`std.service.InternalError`；
- 补齐current WebSocket四个type、四个direct send native、一个
  `requestJsonToConnection` native及两个source JSON helper；
- 补齐current HTTP type/helper inventory；file inventory保持现状；
- 不新增第二份与`std/api.yml`冲突的canonical surface。

聚焦命令：

```bash
node scripts/check-skiff-source-layout.mjs
node scripts/verify.mjs --only checks --list
```

该checker属于default verify，所以K是combined gate。

### D：README owner

依赖：C0；可与所有节点并行，且不阻塞cheap combined。

精确写集：

- `router/README.md`；
- `runtime/README.md`。

更新内容：

- external `http.yml`/`websocket.yml` owner及profile timeout；
- 无raw receive、无business route/automatic response、无业务可见peer id；
- 双向declared JSON-RPC request、notification忽略、`$/cancelRequest`；
- direct/business `connection.send` downlink与RPC captured observed writer的边界；
- GatewayEntry v2、ServiceProtocol v5、DeploymentArtifact v3、RuntimeAssembly v2；
- 删除没有额外current信息的旧publish/build/service-assembly拓扑段落，不做`publication`全文改名。

### 依赖与并行关系

```text
                         +-> R Rust/test-runner fixtures ----+
C0 existing owners -----+-> X cross-system corpus/tool -----+-> cheap combined
                         +-> K source-layout checker --------+
                         +-> D README (non-blocking)
```

R、X、K写集互斥，三者可并行。D也互斥，但不应因README润色延迟combined。

## 5. Combined入口

完成R、X、K后即可进入cheap combined。最低入口证据是：

```bash
node scripts/check-skiff-source-layout.mjs
node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
node cross-system-fixtures/package-service-ecosystem/verify.mjs --combined-probe
node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-package-test --test package_artifact \
  entrypoint_validation_rejects_non_exact_gateway_facts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment
```

X列出的Router/Rust shared-corpus consumer也应在同一cheap combined中运行。D可在其后完成；除非上层
另有显式文档验收，README不属于production gate。

## 6. 本次只读探针

| 命令 | 结果 |
| --- | --- |
| `node scripts/check-skiff-source-layout.mjs` | FAIL：唯一当前输出为`compiler builtin registry must own CancelError` |
| `node .../verify.mjs --runtime-wire-self-test` | 本worktree在载入Router module时因未安装本地`yaml`依赖停止；未修改/链接依赖。直接父Z3D在同一corpus上已到达ServiceProtocol v3→v5首错，当前源码仍精确保留该loop和v3 fixture |
| package-test指定filter | FAIL：两个`collection_name_mapping` `E0063` |
| `cargo test -p skiff-runtime-host --lib` | FAIL：304 passed / 4 failed；四项均由同一assembly v1 fixture decode失败 |
| test-runner指定integration | FAIL：27 passed / 1 failed / 1 ignored；唯一失败为WebSocket fixture PackageBuild golden mismatch |
| targeted test-runner orchestration unit | PASS：1 passed；证明其中DeploymentArtifact v2只是未校验的stale positive，不是当前direct failure |
| obsolete live/helper反向搜索 | 0命中 |
| orphan WebSocket response corpus consumer搜索 | 0命中 |

未安装依赖、未创建临时symlink、未改production/test/fixture/checker/README，未派sub-agent，未
merge、rebase或push。
