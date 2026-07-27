# P5-F431B Runtime connect final mechanical closure

状态：Ready。高风险路径的低语义、四文件最终闭合。

## 直接父节点

- `P5-F431A-runtime-connect-remaining-owner-audit-result.md`

父审计已在连续两次scope expansion后完成全量definition/literal反搜，冻结全部剩余owner、保护面、
compile顺序和allowlist，并继续追溯到唯一权威设计。不存在实现方向未知量。

## 输入与DAG

| commit | tree |
| --- | --- |
| `54ae5c921fc8639164b9259d75fe55d7f91c49de` | `24bdc8965dc51dd7a36b86f52f948542d69ef37a` |

本节点完成F429A/F430A剩余静态闭合；与已完成F429B Router合流后解除D4和Runtime+Router combined
probe。当前不是稳定候选。

## 唯一写入范围与变化

只允许四个文件及本leaf result：

1. `runtime/capability-context/src/outbound_control.rs`
   - 只删除`RequestStartControl::{business_identity,websocket_entry_id}`；
   - 必须保留current `ConnectionSendControl`同名字段。
2. `runtime/host/src/capability_context/outbound_service.rs`
   - 只从`request_start_control` literal删除两个恒`None`初始化。
3. `runtime/transport/src/control_mapper.rs`
   - 从`request_start_frame_header`删除上述两个投影和`websocket_adapter: None`；
   - 从父审计7.1列出的两个direct fixture删除五个stale初始化/断言；
   - 不改`connection_send_frame_header`、current connection-send fixture或payload行为。
4. `runtime/host/src/host/request_trace.rs`
   - 只从embedded `RequestEnvelope` literal删除`websocket_adapter: None`。

全部是机械删除。禁止replacement字段、compat alias、serde default、dual-read或fallback。

## 保护面与停止条件

禁止修改：

- `runtime/transport/src/protocol.rs`、`runtime_assembly_request*`、shared wire corpus；
- admission、activation、eval callable、accept/reject、provider rebinder、generation lifecycle；
- `ConnectionSendControl` / `ConnectionSendFrameHeader` / `ConnectionSendEnvelope`及tests；
- Router、test-runner、compiler/authoring/deployment、std、Internals、skiff-packages。

若四文件之外仍有编译owner或positive legacy命中，停止并返回精确证据；不得再扩大范围。

## 验证

本Agent按父审计7.3顺序唯一执行：

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

`skiff-test-runner`及filtered suite预计仍只受D4 optional-handler三错遮挡；如实记录，不得修改或
伪报PASS。按父审计7.4执行completion反搜，尤其：

- `websocket_adapter`在`runtime/**`、`artifact-model/**`为零；
- generic request.start三owner不再有两个legacy identity字段；
- current connection.send chain和字段仍存在且tests不变。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f431b-runtime-final-closure`
- 分支：`codex/p5-f431b-runtime-final-closure`

启动后5分钟内完成第一次实际修改。提交implementation，再新增并提交
`P5-F431B-runtime-connect-final-mechanical-closure-result.md`。返回commit/tree、完整命令状态、
反搜证据和clean状态。不得merge、rebase、push、stable/live；完成后不得承接D4或combined probe。
