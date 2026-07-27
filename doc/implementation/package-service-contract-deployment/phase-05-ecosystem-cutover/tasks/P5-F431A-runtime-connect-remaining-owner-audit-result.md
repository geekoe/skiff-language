# P5-F431A Runtime connect remaining-owner closure audit result

状态：`AUDIT_COMPLETED / IMPLEMENTATION_TASK_EXECUTABLE`。本次只读审计已经把第三个、且应为最终一个
Runtime closure 的 production/direct-fixture 写集冻结为四个 Rust 文件。没有发现会改变实现方向的
未知量，因此不是 `TASK_NOT_EXECUTABLE`。

本 leaf 没有修改 production、test、fixture 或 generated file，没有运行完整 gate，也没有展开审阅
Router、test-runner 实现、Internals或skiff-packages，更没有访问 stable 或 live。对test-runner只运行
任务要求的直接 compile diagnostic。

## 1. Exact candidate 与审计边界

| 节点 | commit | tree |
| --- | --- | --- |
| frozen implementation candidate | `64e5be20baa253c1b12f2cd2125b22888112e75d` | `7578f4449bd7f3331670007fc36f53ee0ceb848b` |
| audit task checkout | `61923b9bcf0272b9a3f45c2794c3023922102d84` | `0a0535e00db30f8a72629f6aeaab588edd859c7a` |

`64e5be20..HEAD` 只新增
`P5-F431A-runtime-connect-remaining-owner-audit.md`；`runtime/**` 和 `artifact-model/**` 与 frozen
candidate 完全相同。因此下述源码行号和 compile diagnostics 都对应指定 candidate。

审计读取了 F425A/F426A/F429A/F430A 的任务与 result，并只在 `runtime/**`、`artifact-model/**`
内做定义、完整 literal 和 legacy symbol 反搜。

## 2. Generic `request.start` 精确 owner 矩阵

### 2.1 字段级调用链

| 角色 | 精确 owner | candidate 事实 | final closure 动作 |
| --- | --- | --- | --- |
| shared definition | `runtime/capability-context/src/outbound_control.rs::RequestStartControl` | lines 181–182 仍声明 legacy `business_identity`、`websocket_entry_id` | 删除两个字段 |
| control carrier | 同文件 `OutboundControlMessage::RequestStart` | 携带完整 `RequestStartControl` 与 payload；自身没有 legacy 字段 | 不改 variant |
| production producer | `runtime/host/src/capability_context/outbound_service.rs::request_start_control` | 唯一 production 完整 literal；两个字段恒为 `None` | 删除两项初始化 |
| production dispatch | 同文件 `start_request` | 将 producer 结果装入 `OutboundControlMessage::RequestStart` | 不改 |
| transport projection | `runtime/transport/src/control_mapper.rs::encode_outbound_control_message` -> `request_start_frame_header` | lines 232/233 将两个 shared 字段投影，line 239 还初始化已删除的 `websocket_adapter` | 删除三项 |
| wire definition | `runtime/transport/src/protocol.rs::RequestStartFrameHeader` | current definition已经没有这三个字段 | 不改，保持 strict current generic request header |
| downstream consumer | `runtime/transport/src/request_mapper.rs::request_envelope_from_start_frame` 与 `ingress_selector.rs::ingress_selector_from_start_frame` | 不读取三个 legacy 字段；只消费 current HTTP/generic fields | 不改 |

`RequestStartControl` 经
`runtime/{capability-context,request-contract,request}/src/{lib,outbound}.rs` 和
`runtime/driver/capability_context/mod.rs` 重导出；这些都是不检查字段的 type pass-through，不需要
写入。`runtime/eval/src/service_dispatch.rs` 与 driver tests 也只把完整 control 当作 opaque test
结果传递，不是新增 field-sensitive owner。

### 2.2 全部完整 struct literal

`RequestStartControl { ... }` 的完整 literal 只有两个：

| owner | 类型 | legacy 项 | 动作 |
| --- | --- | --- | --- |
| `runtime/host/src/capability_context/outbound_service.rs:292` | production | lines 308–309 两个恒 `None` | 删除 |
| `runtime/transport/src/control_mapper.rs:588` | direct fixture | lines 604–605 两个 `Some` | 删除 |

`RequestStartFrameHeader { ... }` 的完整 literal 共八个：

| owner | 类型 | 状态 |
| --- | --- | --- |
| `runtime/transport/src/control_mapper.rs:217` | production projection | 三个 legacy 项，必须删除 |
| `runtime/transport/src/control_mapper.rs:480` | direct fixture | lines 498/499/510 三个 legacy 项，必须删除 |
| `runtime/transport/src/request_mapper.rs:237` | direct fixture | 已是 current shape，不改 |
| `runtime/transport/src/request_mapper.rs:420` | direct fixture helper | 已是 current shape，不改 |
| `runtime/transport/src/ingress_selector.rs:127` | direct fixture helper | 已是 current shape，不改 |
| `runtime/transport/src/protocol/tests.rs:1047` | direct fixture | 已是 current shape，不改 |
| `runtime/transport/src/protocol/tests.rs:1113` | direct fixture | 已是 current shape，不改 |
| `runtime/transport/src/protocol/tests.rs:1470` | direct fixture | 已是 current shape，不改 |

`control_mapper.rs:639` 还读取不存在的 `decoded.websocket_adapter`；它不是 literal，但会在 production
首错修复后成为 test compile owner，必须删除该 assertion。

### 2.3 必须保护的 current 同名字段

以下不是 legacy generic `request.start` 字段，不能删除或改名：

```text
runtime/host/src/capability_context/websocket.rs::send_connection_frame
  -> capability-context::ConnectionSendControl
  -> OutboundControlMessage::ConnectionSend
  -> transport::control_mapper::connection_send_frame_header
  -> transport::protocol::ConnectionSendFrameHeader
```

这条 current `connection.send` 链必须继续携带：

- activation-owned `service_id`；
- sole-entry `websocket_entry_id`；
- 二选一的 `business_identity` / `connection_id`；
- `payload_kind` 与 opaque payload。

`runtime/transport/src/protocol.rs::{ConnectionSendFrameHeader,ConnectionSendEnvelope}`、
`runtime/capability-context/src/outbound_control.rs::ConnectionSendControl` 以及 current
`RuntimeAssemblyWebSocketConnect*` request/response 中的同名字段也都属于 current 协议。尤其
`control_mapper.rs` 在 final closure 中虽是写入文件，`connection_send_frame_header` 和
`connection_send_frame_maps_header_and_opaque_payload` 仍是显式禁止修改面。

## 3. Deleted DTO 与 `RequestEnvelope` 全 runtime 反搜

### 3.1 仍需写入的全部 residual

| owner | production / fixture | residual | 暴露方式 |
| --- | --- | --- | --- |
| `runtime/capability-context/src/outbound_control.rs` | production definition | `RequestStartControl::{business_identity,websocket_entry_id}` | reverse-search/API 残留；删除后驱动下游 compile |
| `runtime/host/src/capability_context/outbound_service.rs` | production producer | 上述两字段的 `None` literal | shared definition删除后 `E0560` |
| `runtime/transport/src/control_mapper.rs:232/233/239` | production projection | 两个字段投影与 `websocket_adapter: None` | 当前三个 transport `E0560` |
| `runtime/transport/src/control_mapper.rs:498/499/510` | direct fixture | stale `RequestStartFrameHeader` literal | transport test target在production修复后暴露 |
| `runtime/transport/src/control_mapper.rs:604/605` | direct fixture | stale `RequestStartControl` literal | shared definition删除后暴露 |
| `runtime/transport/src/control_mapper.rs:639` | direct fixture | `decoded.websocket_adapter` assertion | wire definition已删，test compile时暴露 |
| `runtime/host/src/host/request_trace.rs:68` | direct fixture | 已删除的 `RequestEnvelope.websocket_adapter` literal | Host test target compile时暴露 |

对所有 `RequestEnvelope { ... }` 完整 literal 的反搜证明，`request_trace.rs:68` 是唯一仍初始化
`websocket_adapter` 的 owner。F429A 指出的两个 driver fixture以及 request/eval/Host 其他 literal
已经在 F430A checkpoint 收口。

### 3.2 已删除 symbol family

对以下精确 legacy family 做 word-boundary 搜索，production 和 positive fixture 命中均为零：

- request DTO：`WebSocketAdapter`、`WebSocketAdapterKind`、`WebSocketContextExpectation`、
  `WebSocketContextCodec`、`WebSocketReceiveRequest`、`WebSocketMessage*`、
  `WebSocketPayloadSegment*`；
- response DTO：`WebSocketResponse`、legacy `WebSocketConnectAccept`、
  `WebSocketConnectContext`、`WebSocketConnectReject`、`ResponseEnd::WebSocket`；
- transport DTO：legacy `RuntimeWebSocketAdapter*`、`RuntimeWebSocketContext*`、
  `RuntimeWebSocketReceive*`、`RuntimeWebSocketMessage*`、`RuntimeWebSocketPayload*` 和
  `RuntimeWebSocketResponseFrameHeader` family；
- eval/boundary/operation：`AdmittedWebSocketIngressIdentity`、
  `dispatch_websocket_ingress_via_in_process_boundary`、`websocket_ingress_context`、
  `WEBSOCKET_INGRESS_OPERATION_NAME`、`WEBSOCKET_INGRESS_EVENT_TYPE` 和
  `WEBSOCKET_CONNECT_RESULT_TYPE`。

当前 `runtime/eval/src/runtime_websocket_connect.rs::RuntimeWebSocketConnect*`、
`GatewayAdapterSource::WebSocketConnectRequest` 和 current assembly wire 类型不是上述 legacy
family，必须保留。

### 3.3 Completion negative allowlist

旧 spelling 的合法剩余只允许 strict-negative evidence：

| owner | 允许命中 |
| --- | --- |
| `artifact-model/src/gateway.rs:680,906` | `websocketReceive` 作为非法 protocol/source spelling |
| `artifact-model/src/gateway.rs:946` | `std.websocket.WebSocketIngressEvent` 作为非法 current connect schema |
| `runtime/transport/src/response_mapper/tests.rs:139` | `contextPayloadPresent` 作为 strict unknown-field rejection |

若 broad search 包含 `websocketEntryId`，还会命中 current assembly connect、activation、generation、
capability、`connection.send` 及其测试；这些全部是 current allowlist。另有
`runtime/request/src/assembly_ingress.rs` 和
`runtime/host/src/host/router_session/tests/runtime_assembly_request.rs` 对 legacy HTTP bridge metadata
的 fail-closed 负例，它们不能被当成 positive behavior residual。

除上述 strict-negative evidence 外，`receiveEvent`、`websocketReceive`、`websocket.receive`、
`contextCodec`、`contextPayloadPresent`、旧 Context/operation DTO 和 `websocket_adapter` 在 final
closure 后必须为零；不得用 alias、default、dual-read 或 fallback 保留。

本节的关键只读反搜入口为：

```bash
rg -n '\bRequestStartControl\s*\{' runtime artifact-model
rg -n '\bRequestStartFrameHeader\s*\{' runtime artifact-model
rg -n '\bRequestEnvelope\s*\{' runtime artifact-model
rg -n 'business_identity|websocket_entry_id|websocket_adapter' runtime artifact-model
rg -n 'WebSocketIngressEvent|WebSocketReceiveEvent|ConnectionMessage|receiveEvent|websocketReceive|websocket\.receive|contextCodec|contextPayloadPresent|websocket_adapter' runtime artifact-model
```

对每个候选都继续读取完整 definition/literal block，排除了函数返回类型、re-export和current
`connection.send`/assembly-connect同名命中；没有把 `rg` 的纯文本命中数当成 owner 数。

## 4. 最小 compile diagnostics

Cargo metadata确认 package id：

```text
skiff-runtime-transport -> runtime/transport/Cargo.toml
runtime                 -> runtime/Cargo.toml
skiff-test-runner       -> test-runner/Cargo.toml
```

| 命令 | candidate 结果 |
| --- | --- |
| `cargo check -p skiff-runtime-transport` | FAIL，且只报告 `control_mapper.rs:232/233/239` 三个 `E0560` |
| `cargo check -p runtime` | FAIL，first blocker仍是同三处 `E0560`；其余输出只有无关既存 warnings |
| `cargo check -p skiff-test-runner` | 独立 FAIL：`canonical_test_gateway.rs:97` 一个 `E0308`；`package_test_assembly.rs:238` 一个 `E0308`；`:241` 一个 `E0277` |
| `git diff --check` | PASS |

因此父 result 的 transport first blocker 精确无误。没有为了“预演修复”临时编辑任何 source；下游
静态 owner 由 definition、field access 与全部完整 literal 反搜冻结为第 3.1 节七行、四文件集合。
不存在等待编译器逐错发现的第五个文件。

D4 也已被单独 package check 证明为独立分支：该命令不经过
`skiff-runtime-transport`，仍精确产生父 result 的三个 optional-handler 错误。transport closure
不会改变它们；反之 D4 修复也不会消除本 leaf 的三个 first blocker。

## 5. Ordinary provider capability rebinder 复核

### 5.1 Production owner chain

| 跳点 | owner | 结论 |
| --- | --- | --- |
| rebinder contract | `runtime/eval/src/capabilities.rs::WebsocketCapabilityRebinder` | owned closure只接收 provider `service_id` 与 optional sole-entry id |
| execution-context retention | `runtime/eval/src/program_execution.rs::ProgramExecutionContext`、`OwnedProgramExecutionContext::{capture,borrow}` | clone、owned stream capture与borrow均保留 rebinder |
| provider switch | `runtime/eval/src/assembly_execution/async_stream_cancel.rs::provider_execution_context` | 从 `provider_activation()`读取 provider service/entry，先 rebind，再切 provider assembly target |
| concrete factory | `runtime/host/src/eval_capability_adapter/factory.rs::{websocket_from_request,websocket_rebinder}` | sender被owned clone；不读取 caller owner补默认 |
| top-level HTTP/connect | `runtime/host/src/eval_capability_adapter/assembly_request_adapter.rs::program_execution_context` | HTTP 与 connect 共用的current assembly adapter注入 rebinder |
| top-level actor | `runtime/host/src/eval_capability_adapter/actor_method_adapter.rs::context` | actor execution同样注入 rebinder |
| final control owner | `runtime/host/src/capability_context/websocket.rs::send_connection_frame` | 从 rebound context 构造 current `ConnectionSendControl` |
| final wire projection | `runtime/transport/src/control_mapper.rs::connection_send_frame_header` | 原样投影 provider service/entry与target；不涉及 generic request fields |

version/build没有进入 rebinder或 business fan-out key。四个 native 仍由既有 native/contract owner
提供，签名和 `may_suspend=false` 未改变。

### 5.2 Direct tests

`runtime/host/src/eval_capability_adapter/factory.rs` 三个入口齐全：

1. `provider_rebind_replaces_different_caller_owner_in_control_frame`：caller/provider entry不同，
   断言最终 `ConnectionSendControl` 使用 provider service/entry；
2. `provider_without_entry_makes_all_four_websocket_natives_unavailable`：provider零entry，四个 native
   全部 error 且不发送 frame；
3. `provider_entry_is_available_when_caller_has_no_entry`：caller零entry/provider有entry，provider
   binary-to-business send成功且 control使用provider owner。

`runtime/eval/src/assembly_execution/async_stream_cancel.rs::
ordinary_service_call_rebinds_websocket_capability_to_provider_activation` 还覆盖真实 ordinary
provider target switch，以及 `OwnedProgramExecutionContext` capture/borrow 后 owner不变。
transport已有 `connection_send_frame_maps_header_and_opaque_payload`，protocol tests覆盖 current
header/envelope JSON。

未发现还需要写入但未授权的 provider production/test owner。唯一需要特别保护的是
`control_mapper.rs` 的 current `connection.send` projection：final closure因相邻 generic
`request.start` 残留而写该文件，但不能顺手改动 current chain。

## 6. Current connect 关键跳点与遮挡关系

| 跳点 | production owner | 已有 test source / 历史证据 | candidate 遮挡 |
| --- | --- | --- | --- |
| assembly admission | `runtime/host/src/loader/active_assembly_context.rs::{deployment_websocket_entry,admitted_websocket_entry}` | `websocket_admission_accepts_zero_or_one_exact_entry`；multiple entry/selector、dangling/key、identity/surface负例 | embedded Host tests先被transport，再被D4挡住 |
| activation sole entry | `runtime/activation/src/context.rs::ActivationWebSocketEntry`、`new_with_websocket_entry`、`websocket_entry_matches` | `activation_context_websocket_entry_is_typed_optional_and_matches_all_exact_facts`；F430A独立 activation suite已PASS | 当前可独立执行，不依赖transport/D4 |
| Host current request | `runtime/host/src/host/request_entry/assembly_wire.rs::{spawn_runtime_assembly_request,websocket_connect_request_from_wire,validate_websocket_connect_route}` -> `assembly.rs::spawn_websocket_connect_on_active_assembly_route` | F426A shared current wire corpus曾PASS；`runtime/request/src/websocket_connect_execution.rs` exact header/mismatch tests覆盖assembly、generation、gateway、entry；无单一Host E2E connect test | transport first blocker挡request/Host test compile；Host随后还受D4 |
| exact eval callable | `runtime/request/src/websocket_connect_target.rs::RuntimeAssemblyWebSocketConnectTarget` -> `runtime/eval/src/runtime_websocket_connect.rs::execute_runtime_websocket_connect` | `websocket_connect_target_requires_real_handler_and_exact_plan`；request header tests；eval accept/reject strict decoder tests | request tests先受transport；eval test binary受独立D4 |
| accept/reject mapper | `runtime/host/src/host/request_entry/assembly.rs::websocket_connect_result_into_message` -> `runtime/transport/src/response_mapper.rs::runtime_assembly_websocket_connect_response_into_frame` | eval accept/reject/optional policy tests；F426A response golden/mutation corpus曾PASS | current transport lib首错挡response mapper tests；Host又受D4 |
| generation pin | `assembly.rs::queue_websocket_generation_acquire` -> `runtime/host/src/host/websocket_generation.rs::WebSocketGenerationRegistry` | `websocket_generation_old_route_survives_reload_until_disconnect_without_artifact_io`、exact release/idempotency/fail-closed、acquire rejection rollback | Host tests先受transport，再受D4 |

行为上的 fail-closed 顺序是确定的：

- wire/header/route admission失败时不会构造 eval target；
- missing handler或target fact mismatch在eval前失败；
- eval reject与generic error不获取 generation pin；
- accept先 `begin_acquire` 并成功 queue lifecycle frame，才发送 connect response；
- acquire encode/send失败会 rollback tentative pin并发送 error，而不是 accept；
- route pin持有 exact generation，release/disconnect前不跟随current assembly pointer。

因此 production flow本身没有待选择的分支。当前缺的是四文件机械 compile/reverse-search closure；上游
compile失败遮挡的是既有 test source执行，不是另一项行为设计。

## 7. 第三个、且应为最终一个 implementation 合同

### 7.1 Exact write set

production/test只允许以下四个文件：

```text
runtime/capability-context/src/outbound_control.rs
runtime/host/src/capability_context/outbound_service.rs
runtime/transport/src/control_mapper.rs
runtime/host/src/host/request_trace.rs
```

另允许新增该 implementation 自己的 leaf result；不需要修改任何其他 Rust file或fixture。

逐文件变化：

1. `outbound_control.rs`：只从 `RequestStartControl` 删除
   `business_identity`、`websocket_entry_id`；保留 `ConnectionSendControl` 同名字段；
2. `outbound_service.rs`：只从 `request_start_control` 完整 literal删除两个恒 `None` 初始化；
3. `control_mapper.rs`：
   - 从 `request_start_frame_header` 删除两个投影和 `websocket_adapter: None`；
   - 从 `request_start_frame_maps_header_and_opaque_payload` fixture删除三个 stale字段；
   - 从 `outbound_request_start_control_encodes_binary_frame` 的 `RequestStartControl` literal删除两个
     stale字段，并删除 `decoded.websocket_adapter` assertion；
   - 不改 `connection_send_frame_header`、current connection-send fixture或任何 payload行为；
4. `request_trace.rs`：只从 embedded `RequestEnvelope` literal删除
   `websocket_adapter: None`。

全部是机械闭合；不得加入 replacement字段、兼容 alias、serde default、dual-read或fallback。

### 7.2 明确禁止面

- 不改 `runtime/transport/src/protocol.rs` current wire definition；
- 不改 `runtime_assembly_request*` 或 shared current wire corpus；
- 不改 admission、activation、eval callable、accept/reject、provider rebinder、generation lifecycle；
- 不改 current `ConnectionSendControl` / `ConnectionSendFrameHeader` / `ConnectionSendEnvelope` 及其测试；
- 不改 Router、test-runner、compiler/authoring/deployment、std、Internals或skiff-packages；
- 不运行 full gate，不承接 D4 或 Runtime+Router combined probe；
- 不 merge、rebase、push、stable/live或操作本机instance。

### 7.3 便宜验证顺序

先用 structural reverse search确认四文件已全部收口，再按下列顺序执行：

```bash
cargo check -p skiff-runtime-transport
cargo test -p skiff-runtime-transport
cargo check -p skiff-runtime-host
cargo check -p runtime
cargo check -p skiff-test-runner
cargo test -p skiff-runtime-request -p skiff-runtime-eval -p skiff-runtime-host websocket
cargo fmt --all -- --check
git diff --check
```

含义：

- 第一个 check 先证明三个first blocker归零；
- transport tests随后立即暴露同文件的完整 literal/assertion遗漏，不等Host或D4；
- Host/root checks再证明 shared carrier与production producer完整；
- direct `skiff-test-runner` check应继续只报告已知D4三个错误，直到D4 owner集成；本任务不得修；
- 最后一个 filtered三package suite只在静态闭合后运行。若D4尚未集成，精确记录其遮挡；D4集成后用
  同一命令执行 admission/request/eval/provider/generation 的 `websocket` test source；
- Runtime+Router combined probe仍由DAG后继唯一执行。

### 7.4 Completion reverse-search allowlist

completion必须同时满足：

1. `websocket_adapter` 在 `runtime/**`、`artifact-model/**` 为零；
2. 第3.2节所有已删除 exact symbol在production/positive fixture为零；
3. `RequestStartControl`、Host `request_start_control` 与
   `control_mapper::request_start_frame_header` 均不再出现
   `business_identity`/`websocket_entry_id`；
4. 旧 receive/context/operation spelling只剩第3.3节 strict-negative evidence；
5. 同名 current命中只允许：
   - `RuntimeAssemblyWebSocketConnect*` request/response；
   - activation/admission/provider capability/generation owners；
   - `ConnectionSendControl`、`ConnectionSendFrameHeader`、`ConnectionSendEnvelope`、
     `connection_send_frame_header`及其direct tests；
   - current schema/strict-negative tests。

任一新增文件、positive legacy命中或 current `connection.send` 变化都应停止并重新审计；按本次全量
definition/literal反搜，正常实现不应再次 scope-expand。

## 8. 审计结论

`TASK_NOT_EXECUTABLE` 条件未触发。剩余工作没有协议、identity、routing、generation或provider-owner
设计选择；只有四文件机械删除与既有D4遮挡。第三个 Runtime closure可按第7节一次完成，完成后本
Agent不承接 implementation、D4或 combined probe。
