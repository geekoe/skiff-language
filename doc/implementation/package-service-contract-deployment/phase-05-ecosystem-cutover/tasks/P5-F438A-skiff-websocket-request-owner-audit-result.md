# P5-F438A Skiff WebSocket request owner audit result

状态：`TASK_NOT_EXECUTABLE`

## 审计基线

| 项目 | commit | tree |
| --- | --- | --- |
| 权威设计输入 | `64a0ab4ec85d25899dc8563ac6d647edad8ed23e` | `562adcfc8baa595969a4dd1ccd2e67c4053814b9` |
| 审计起点 | `f74404fbd466e96005a750fbb5b4ccae165cc401` | `4902066652382289d1282536ff6be0885b2cd7a0` |

权威设计输入是审计起点的祖先。两者之间只有F437B/F438/F438A/F438B任务编排文档，没有
production、test或fixture实现差异。

## 最小决策问题

请冻结`std.websocket.WebSocketRequestError`的公开投影表：

1. 平台产生的以下failure分别使用什么稳定`code`，以及`message`和`detail`的固定、脱敏形状：
   connection不存在/已关闭、caller cancel、execution deadline、platform protocol failure、
   pending/payload/tombstone limit拒绝、Router或原runtime断线。
2. peer返回
   `{ "ok": false, "error": { "code": string, "message": string, "detail": Json? } }`
   时，公开错误是原样透传、限长/脱敏后透传，还是改写为平台固定code并把peer error放入`detail`。

这是公共错误语义，不是内部命名选择。`code`会被Skiff业务代码读取并分支；不同答案会改变Router
`connection.response`错误体、runtime carrier/catch projection、聚焦测试和Host兼容行为，不能由实现
agent自行选择。

权威设计已经冻结：

- public字段为`code: string`、`message: string`、`detail: Json?`；
- connection、deadline、cancel、platform protocol和peer显式error都投影为
  `std.websocket.WebSocketRequestError`；
- typed request encode和success response decode仍投影为`std.json.DecodeError`。

但权威设计没有冻结上述code词汇或peer error投影策略。当前production也没有可继承的owner：

```text
rg -n 'requestJsonToConnection|WebSocketRequestError|connection\.request|connection\.response|connection\.request\.cancel' \
  std compiler runtime router test-runner scripts cross-system-fixtures
=> 0 matches
```

因此本leaf按任务第8条停止，不输出owner矩阵、wire字段冻结或后继实现DAG；在公共错误投影表确定前，
这些交付会把未获授权的设计选择伪装成审计结论。

## 只读证据与遮挡

- `cargo metadata --no-deps --format-version 1`：命中29个`skiff-artifact-model`、
  `skiff-compiler-*`和`skiff-runtime-*`package。
- 当前send链反搜：
  `rg -l 'std\.websocket\.send(Text|Binary)To(Connection|BusinessIdentity)|connection\.send|ConnectionSend' ...`
  命中33个production/test/README文件。
- schema反搜：
  `rg -l 'RUNTIME_FRAME_SCHEMA_VERSION|skiff-runtime-frame-v1' runtime router cross-system-fixtures`
  命中75个文件。
- test listing：compiler 1 phase、runtime 3 phases、Router 1 phase，均只listing，未运行完整gate。
- `cargo test -p skiff-runtime-transport connection_send_frame_maps_header_and_opaque_payload`：
  selector非零，1 passed、0 failed、81 filtered。
- `node scripts/check-skiff-source-layout.mjs`：通过。
- `cargo test -p skiff-runtime-eval connection_send_stays_inside_the_current_synchronous_segment`：
  selector在源码中存在，但test target先被既存
  `runtime/eval/src/runtime_http_gateway/tests.rs:384`
  的`Option<PackageCallableId>.as_str()`编译错误遮挡。
- `node cross-system-fixtures/package-service-ecosystem/verify.mjs`：未启动fixture断言；
  当前worktree缺少Router `yaml` package。只读审计未安装依赖。
- 未运行完整Rust/Router suite、live、instance或stable；未安装依赖，未修改production/test/fixture/design。
