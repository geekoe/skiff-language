# P5-F430A Runtime WebSocket connect closure result

状态：`TASK_SCOPE_EXPANDED / SAFE CHECKPOINT`。授权范围内已经删除大部分D2 legacy
receive/message/Context/operation consumer，完成ordinary provider activation的WebSocket capability
rebind，并补入connect/admission/native相关direct tests；但最终编译闭合发现
`runtime/transport/src/control_mapper.rs`和共享`RequestStartControl`是任务未授权的新production
owner。按停止条件已撤回范围外修改，不把本结果判为完成或稳定候选。

## 1. Exact candidate与implementation checkpoint

| 节点 | commit | tree |
| --- | --- | --- |
| frozen input | `bbe5233d71dc143458371aa366dbc385c7dcd261` | `0344ee467060fb374c5b3f80f7ebe478cd734c3a` |
| task checkout | `ddac18f410a7d13eb3ec214af701151bf40e43bd` | `f8f8e11c9470647f17244c5a61fccd84f01b2c67` |
| implementation checkpoint | `5362232ddb515f7261a28409b230a5291b9c5a13` | `aa7d043acba8247423b14508ff316c8daea0ff46` |

task checkout相对frozen input只增加本任务合同。F426A current wire owner
`runtime/transport/src/runtime_assembly_request.rs`在task checkout和implementation checkpoint中的
blob均为`36686cd6a6280d49f260468fac84321dd8881d76`，没有修改current wire/corpus。

## 2. 授权范围内形成的checkpoint

### 2.1 D2 legacy consumer删除

- 删除`artifact-model::websocket_ingress` compatibility API及re-export；
- 删除runtime boundary、linked type plan中的legacy WebSocket shape descriptor与runtime plan；
- 删除eval legacy websocket adapter、contract/identity/ingress/response execution owner及driver shim；
- 删除request/request-contract的legacy ingress、response和adapter branches；
- 删除旧generic transport request/response DTO、selector和mapper branches；
- `http_response_ceiling`不再匹配legacy `ResponseEnd::WebSocket`，current connect仍只走专用
  current mapper。

### 2.2 Current connect与admission evidence source

- admission精确验证零或一个entry、selector/key join、canonical gateway identity/surface、handler、
  adapter plan和internal entry id；
- activation direct test覆盖typed optional sole-entry record及selector/key/gateway identity/internal
  entry id全部mismatch；
- request direct tests覆盖exact header、activation/assembly/generation/host/gateway/internal entry
  mismatch和无handler target；
- eval decoder direct tests覆盖accept/reject、optional identity/policy及strict-invalid shape；
- 既有current wire corpus继续覆盖空payload、strict mutation、accept/reject response与generation
  lifecycle。

### 2.3 Ordinary provider capability rebind

`ProgramExecutionContext`现在携带owned `WebsocketCapabilityRebinder`。ordinary in-process service
call切到provider activation时，先从provider service id与typed sole-entry重建capability，再替换
runtime assembly target；owned stream capture继续保留该rebinder。

Host HTTP/connect/actor顶层adapter均注入由router sender构造的rebinder。新增direct test source覆盖：

- caller/provider都有不同entry，control frame使用provider service/entry；
- provider无entry，四个native均unavailable且不发送frame；
- caller无entry、provider有entry时provider native可用；
- ordinary provider context及owned capture保留provider owner。

rebinder输入只有service id与sole-entry id，version/build不进入business fan-out key；四个native
签名和`may_suspend=false`未改。

## 3. TASK_SCOPE_EXPANDED：新增production owner

### 3.1 精确编译遮挡

授权内`runtime/transport/src/protocol.rs`删除legacy generic
`RequestStartFrameHeader::{business_identity,websocket_entry_id,websocket_adapter}`后，范围外
`runtime/transport/src/control_mapper.rs::request_start_frame_header`仍构造这三个字段：

```text
runtime/transport/src/control_mapper.rs:232 business_identity
runtime/transport/src/control_mapper.rs:233 websocket_entry_id
runtime/transport/src/control_mapper.rs:239 websocket_adapter
```

`cargo check -p skiff-runtime-transport`因此产生三个`E0560`。该文件不在任务列出的
`runtime/transport/src/{ingress_selector,protocol,request_mapper,response_mapper}.rs`写集内，已撤回
诊断期间的机械删除。

### 3.2 四个闭合点、调用链与最小新增写集

| 闭合点 | 精确owner | 所需机械变化 | 当前授权 |
| --- | --- | --- | --- |
| 1. shared control carrier | `runtime/capability-context/src/outbound_control.rs::RequestStartControl` | 删除`business_identity`、`websocket_entry_id`两个legacy generic request字段 | 新增production写集 |
| 2. Host producer literal | `runtime/host/src/capability_context/outbound_service.rs::request_start_control` | 删除两个恒为`None`的字段初始化 | 已在原Host capability写集 |
| 3. transport production projection | `runtime/transport/src/control_mapper.rs::request_start_frame_header` | 删除上述两个字段投影及`websocket_adapter: None` | 新增production写集 |
| 4. direct fixture literals | `control_mapper.rs` lines 498/499/510/639；`runtime/host/src/host/request_trace.rs:68` | 删除旧字段初始化与旧adapter assertion | `control_mapper` tests随新增owner；`request_trace.rs`需新增fixture写集 |

精确调用链为：

```text
runtime/host/src/capability_context/outbound_service.rs
  ::request_start_control
  -> runtime/capability-context::RequestStartControl
  -> OutboundControlMessage::RequestStart
  -> runtime/transport/src/control_mapper.rs
     ::encode_outbound_control_message
     ::request_start_frame_header
  -> runtime/transport::RequestStartFrameHeader
```

另一个fixture闭合链为：

```text
runtime/request-contract::RequestEnvelope.websocket_adapter（已删除）
  -> runtime/host/src/host/request_trace.rs embedded test完整literal
```

这属于纯机械闭合，不引入新wire或routing语义：

- Host production对两个shared control字段只写`None`；
- generic `RequestStartFrameHeader`已经不再声明这三个字段；
- current WebSocket connect使用独立`runtimeAssembly.websocketConnect` frame；
- current downlink `ConnectionSendControl/ConnectionSendFrameHeader`中的service、entry、
  business identity和connection id是当前协议，不应删除。

仅从`control_mapper`删除投影虽可恢复编译，但会把两个无生产意义的shared control字段保留为
compatibility API，违反本任务“不能保留alias或dual-read”的要求。因此后继最小新增production写集
必须同时包含：

```text
runtime/capability-context/src/outbound_control.rs
runtime/transport/src/control_mapper.rs
```

并加入`runtime/host/src/host/request_trace.rs`作为direct fixture闭合owner；原已授权的
`runtime/host/src/capability_context/outbound_service.rs`继续随字段删除机械收口。

## 4. 验证矩阵

### 4.1 当前checkpoint实际PASS

| 命令 | 结果 |
| --- | --- |
| `cargo test -p skiff-artifact-model -p skiff-runtime-activation -p skiff-runtime-boundary -p skiff-runtime-linked-type-plan -p skiff-runtime-loader -p skiff-runtime-linker -p skiff-runtime-request-contract -p skiff-runtime-native -p skiff-runtime-native-contract` | PASS；共540 tests |
| `cargo check -p skiff-runtime-eval` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

其中native suite继续验证四个current WebSocket native registry/signature事实；linked-type-plan删除
legacy shape后17个tests全部通过。

### 4.2 精确BLOCKED

| 命令/目标 | 结果 |
| --- | --- |
| `cargo check -p skiff-runtime-transport` | BLOCKED；`control_mapper.rs:232/233/239`三个`E0560`，见第3节 |
| 任务规定的13-package combined `cargo test` | BLOCKED；同时命中上述transport scope blocker与既有D4 `skiff-test-runner` blocker |
| `skiff-runtime-request` / `skiff-runtime-host` tests | BLOCKED；先经过`skiff-runtime-transport` |
| `skiff-runtime-eval` tests | BLOCKED；`skiff-test-runner`在生成eval test binary前失败 |
| `cargo check -p skiff-runtime-driver` | BLOCKED；workspace没有该package id |
| `cargo check -p runtime`（实际root package） | BLOCKED；经过`skiff-runtime-transport`命中第3节 |

D4精确错误保持不变，且本任务未修改`test-runner`：

```text
test-runner/src/canonical_test_gateway.rs:97
  expected Option<PackageCallableId>, found PackageCallableId
test-runner/src/package_test_assembly.rs:238
  expected Option<PackageCallableId>, found PackageCallableId
test-runner/src/package_test_assembly.rs:241
  Option<PackageCallableId> does not implement Display
```

因此provider-rebind、admission和eval connect direct test源码已经落地，但Host/eval source execution
不能在当前checkpoint伪报PASS。

## 5. Reverse search

对`WebSocketIngressEvent`、`WebSocketReceiveEvent`、`ConnectionMessage`、
`WebSocketConnection<`、`WebSocketConnectResult<`、`receiveEvent`、`websocketReceive`、
`websocket.receive`、`contextCodec`、`contextPayloadPresent`和`websocket_adapter`反向搜索：

- 授权D2 production owner中的旧receive/message/Context/operation实现已归零；
- `artifact-model/src/gateway.rs`剩余三个strict-negative assertions；
- `runtime/transport/src/response_mapper/tests.rs`剩余一个
  `contextPayloadPresent` strict-negative fixture；
- 范围外残留精确为`control_mapper.rs` production line 239及其direct tests
  lines 510/639、`request_trace.rs:68` direct fixture；
- shared generic carrier残留另由
  `RequestStartControl::{business_identity,websocket_entry_id}`及Host恒`None` producer构成，见第3节；
- current `websocketConnect`及current downlink identity字段不属于legacy命中。

因此reverse-search门禁尚未完成，不能解除D2。

## 6. 合同验收状态

| 必须完成项 | 状态 |
| --- | --- |
| sole-entry admission、typed activation、current connect execution | IMPLEMENTED CHECKPOINT；独立activation/request-contract evidence PASS，downstream execution受scope blocker |
| 删除全部D2 legacy consumer/alias | BLOCKED / TASK_SCOPE_EXPANDED |
| driver shim与HTTP ceiling机械闭合 | IMPLEMENTED |
| ordinary provider capability rebind | IMPLEMENTED；Host/eval direct execution受D4/scope blocker |
| admission完整负例 | TEST SOURCE ADDED；Host suite blocked |
| connect accept/reject、optional fields、mismatch、empty/stale generation | TEST SOURCE / EXISTING CURRENT CORPUS PRESENT；combined blocked |
| 四个native签名与service + sole entry control | PASS于独立native suite；provider-owner Host tests blocked |

## 7. Scope与生命周期

- 未保留`control_mapper.rs`或`request_trace.rs`的越界修改；
- 未修改F426A current wire/corpus、Router、compiler/authoring/deployment producer、
  test-runner、std、Internals或skiff-packages；
- 未merge、rebase、push、stable/live或操作本机instance；
- implementation与本result分开提交；
- 本结果只交付safe checkpoint与精确扩张合同，不解除F430A、D2、D4或combined probe。
