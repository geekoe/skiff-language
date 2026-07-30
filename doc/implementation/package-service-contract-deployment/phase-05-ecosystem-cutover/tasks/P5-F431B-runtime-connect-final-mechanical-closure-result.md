# P5-F431B Runtime connect final mechanical closure result

状态：`COMPLETED`。父审计冻结的四文件 mechanical closure 已全部落地，没有发现第五个编译 owner、
positive legacy 命中或新的设计未知量。generic `request.start` 的 legacy identity/adapter 残留已归零；
current `connection.send` control、wire projection、direct fixture 与 payload 行为保持不变。

本 leaf 没有承接 D4 或 Runtime+Router combined probe，也没有修改其 owner。任务预告的两个 D4
遮挡命令继续只报告同三个 optional-handler 编译错误；它们不使本 leaf 扩大范围。

## 1. Exact candidate 与 implementation

| 节点 | commit | tree |
| --- | --- | --- |
| frozen input | `54ae5c921fc8639164b9259d75fe55d7f91c49de` | `24bdc8965dc51dd7a36b86f52f948542d69ef37a` |
| task checkout | `73ae5203899fea850a718bc2cac601f242366f57` | `2ea4888da1305a0feab2cfc28650ccba823bb5c3` |
| implementation | `e49e1f2662a959b9594fa67429ae69f3751cdc9a` | `2c7d9d98e8af7bacbd780fc2766761f2ada0dd15` |

task checkout 相对 frozen input 只增加本任务合同。implementation 精确修改四个授权文件，共
14 行纯删除：

```text
runtime/capability-context/src/outbound_control.rs
runtime/host/src/capability_context/outbound_service.rs
runtime/transport/src/control_mapper.rs
runtime/host/src/host/request_trace.rs
```

没有修改 `runtime/transport/src/protocol.rs`、`runtime_assembly_request*`、shared wire corpus、
admission、activation、eval callable、accept/reject、provider rebinder、generation lifecycle、
Router、test-runner、compiler/authoring/deployment、std、Internals 或 skiff-packages。

## 2. Mechanical closure

### 2.1 Generic `request.start`

- `RequestStartControl` 删除 `business_identity` 与 `websocket_entry_id`；
- Host `request_start_control` producer 删除两个恒 `None` 初始化；
- `request_start_frame_header` 删除两个 identity 投影及 `websocket_adapter: None`；
- 两个 `control_mapper` direct fixtures 删除五个 stale 字段初始化和
  `decoded.websocket_adapter` assertion；
- `request_trace` embedded `RequestEnvelope` fixture 删除
  `websocket_adapter: None`。

没有加入 replacement 字段、compatibility alias、serde default、dual-read 或 fallback。

### 2.2 Current `connection.send` 保护

以 task checkout `73ae5203` 为 baseline，逐块 `diff -u` 比较并确认以下内容 byte-identical：

- `ConnectionSendControl` definition；
- `connection_send_frame_header`；
- `connection_send_frame_maps_header_and_opaque_payload` direct fixture。

`runtime/transport/src/protocol.rs` 整文件相对 baseline 无 diff；implementation diff 中也没有
任何 `connection_send` / `ConnectionSend` 增删行。current chain 仍完整保留：

```text
host::send_connection_frame
  -> ConnectionSendControl
  -> connection_send_frame_header
  -> ConnectionSendFrameHeader
  -> ConnectionSendEnvelope
```

其中 service id、WebSocket entry id、business identity / connection id、payload kind 与 opaque
payload 均未改变。transport suite 中
`connection_send_frame_maps_header_and_opaque_payload` 与 protocol 的 current header/envelope
tests 实际 PASS。

## 3. 验证矩阵

严格按父审计 7.3 顺序执行：

| 命令 | 结果 |
| --- | --- |
| `cargo check -p skiff-runtime-transport` | PASS |
| `cargo test -p skiff-runtime-transport` | PASS；82 unit + 2 integration tests，0 failed；doc tests 0 |
| `cargo check -p skiff-runtime-host` | PASS；只有既存 dead-code warnings |
| `cargo check -p runtime` | PASS；只有既存 warnings |
| `cargo check -p skiff-test-runner` | EXPECTED BLOCKED BY D4；仅三个既知错误，见下 |
| `cargo test -p skiff-runtime-request -p skiff-runtime-eval -p skiff-runtime-host websocket` | EXPECTED BLOCKED BY D4；同三个错误，未伪报 tests PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

D4 遮挡精确保持为：

```text
test-runner/src/canonical_test_gateway.rs:97
  E0308: expected Option<PackageCallableId>, found PackageCallableId
test-runner/src/package_test_assembly.rs:238
  E0308: expected Option<PackageCallableId>, found PackageCallableId
test-runner/src/package_test_assembly.rs:241
  E0277: Option<PackageCallableId> does not implement Display
```

两个被遮挡命令没有出现本 leaf 的新编译错误；本任务按合同停止于 D4 边界，没有修改
`test-runner`。

## 4. Completion reverse search

### 4.1 Zero residual

- `websocket_adapter` 在 `runtime/**`、`artifact-model/**`：`ZERO`；
- 父审计 3.2 列出的 deleted exact symbol families：`ZERO`；
- `RequestStartControl` definition、Host `request_start_control` 与
  `control_mapper::request_start_frame_header` 三个 exact block 内的
  `business_identity|websocket_entry_id`：`ZERO`；
- 全部 `RequestStartControl { ... }`、`RequestStartFrameHeader { ... }` 与
  `RequestEnvelope { ... }` literals 已重新枚举，没有发现新增 residual owner。

### 4.2 Strict-negative allowlist

旧 receive/context/operation spelling 只剩父审计允许的四条 strict-negative evidence：

```text
artifact-model/src/gateway.rs:680   websocketReceive invalid spelling
artifact-model/src/gateway.rs:906   websocketReceive invalid spelling
artifact-model/src/gateway.rs:946   std.websocket.WebSocketIngressEvent invalid schema
runtime/transport/src/response_mapper/tests.rs:139
                                    contextPayloadPresent unknown-field rejection
```

`WebSocketReceiveEvent`、`ConnectionMessage`、`receiveEvent`、`websocket.receive`、
`contextCodec` 与 `websocket_adapter` 没有其他命中。

### 4.3 Current same-name allowlist

对 `business_identity|websocket_entry_id` 的全量 `runtime/**`、`artifact-model/**` 反搜逐项复核；
剩余命中只属于 current assembly WebSocket connect、activation/admission/provider
capability/generation、四个 current native、`ConnectionSendControl` /
`ConnectionSendFrameHeader` / `ConnectionSendEnvelope` 及其 direct tests，或 current
schema/strict-negative tests。generic `request.start` 三个 owner 不在剩余命中中。

## 5. Scope 与生命周期

- 没有发现或修改新的 owner；
- implementation 与本 result 分开提交；
- 没有 merge、rebase、push、stable/live、instance 或本机服务操作；
- 没有运行或承接 Runtime+Router combined probe；
- 没有承接 D4；D4 集成后应由其既定 owner/后继重跑被遮挡的 filtered suite；
- implementation 提交后 worktree clean；本 result 是第二个提交的唯一文件。

本 leaf 只解除 F431B 的 Runtime mechanical closure，不宣称 D4、combined probe 或阶段 gate 完成。
