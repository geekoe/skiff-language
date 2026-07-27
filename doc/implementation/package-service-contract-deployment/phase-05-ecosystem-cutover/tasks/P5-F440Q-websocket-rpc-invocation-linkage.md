# P5-F440Q WebSocket RPC invocation linkage

状态：Ready。F440P 的最小 T0 continuation；只关闭 linked owner 与 outbound Host capability 接线。

## 直接父节点

- `P5-F440P-websocket-rpc-transport-checkpoint-result.md`
- `P5-F440B-bidirectional-websocket-owner-audit-result.md`
- `P5-F440O-bidirectional-rpc-prerequisite-gate-result.md`

F440P 已冻结public std ABI、五分支ordinary error carrier、`connection.*` wire、
`ConnectionRequestRegistry`与Router captured-source boundary，但证明current invocation缺少caller-linked
`std.websocket.WebSocketRequestError` owner。本文只完成该共享调用接线；语义继续由F440B和current
reference拥有。

实现基线为`c2abd2e8`对应的当前integration tree。

## DAG位置与目标

本节点完成T0仍有效的最后共享边界，并解除完整E0与Host selectors：

1. `NativeCallPlan`对`requestJsonToConnection`携带caller-linked exact
   `std.websocket.WebSocketRequestError` named-union owner；
2. linked-plan从当前linked executable的真实std symbol解析exact `TypeAddr`，缺失、歧义、错误kind均
   fail closed，不使用global、固定地址或platform builtin；
3. owner经`RuntimeNativeInvocation`传到本次native dispatch；不得从共享capability对象猜调用者；
4. Host WebSocket capability把当前runtime session的registry、captured Router writer、
   cancellation与effective deadline接到F440P已冻结的request future；
5. ordinary response在exact owner下投影五个named-union branch；local JSON codec仍为
   `std.json.DecodeError`，deadline仍为`TimeoutError`，ancestor cancel仍不可捕获；
6. S0新增的WebSocket JSON-RPC gateway kind/source在旧HTTP/connect evaluator中具有明确、正确的
   fail-closed分支，使Host真正编译；本节点不实现inbound adapter。

完成后是T0完整实现检查点，不是E0/R0稳定候选。

## 唯一写集

- `runtime/native-contract/src/call_plan.rs`及其直接tests
- `runtime/linked-type-plan/src/native_call_plan.rs`及其直接tests
- `runtime/native/src/dispatch/{invocation.rs,websocket.rs}`及直接tests
- `runtime/eval/src/native_invocation.rs`
- `runtime/eval/src/capabilities.rs`
- `runtime/eval/src/runtime_http_gateway.rs`
- `runtime/eval/src/runtime_websocket_connect.rs`
- 上述eval文件的colocated tests
- `runtime/host/src/capability_context/native_projection.rs`
- `runtime/host/src/eval_capability_adapter/{websocket.rs,factory.rs,mod.rs}`
- F440P新增的Host connection-request tests所需机械call-site
- 本leaf result

禁止修改std/public ABI、artifact gateway schema、wire/registry语义、`runtime/request`、新
`runtime_websocket_jsonrpc*`、Host request-entry/loader、Router、fixture、test-runner、scripts、其它
task/result。不得派子Agent。

## Exact owner与调用边界

owner必须属于当前linked program/executable，并精确指向public
`std.websocket.WebSocketRequestError` named union。call plan对其它native不增加伪owner；只有需要该
ordinary error surface的binding携带它。

禁止：

- 把union或branch注册为`PlatformBuiltinErrorIdentity`；
- 从symbol name在执行时做global lookup；
- 把一个owner缓存在跨call/caller共享capability上；
- 退化为`ProviderUnavailable`、字符串错误或错误的local `TypeAddr`；
- owner缺失时仍发送peer request。

F440P的默认capability `Unsupported`与missing-owner `InvalidArtifact` fail-closed测试必须保留；production
attached invocation同时具备Host future与exact owner后才允许发送。

## Host request生命周期

接线必须复用F440P的`ConnectionRequestRegistry`：

- captured runtime session与Router writer来自当前Host/router session；
- lease/timer/cancel-on-drop先安装后入队；
-effective deadline取当前invocation deadline与native request policy的严格结果，不自行延长；
-cancel/deadline先settle并释放pending/lease/timer，再best-effort发送专用cancel；
-disconnect/reconnect fencing、late/duplicate response规则不变；
-Host tests结束三类计数均为0。

旧HTTP gateway与connect evaluator遇到仅属于未来JSON-RPC request adapter的kind/source必须明确拒绝为
invalid target/source，不能`unreachable!`、panic、构造空值或把它当raw receive/connect参数。

## 测试先行与验证

先增加至少一项真实red：

- current call plan缺exact owner；或
- wrong/missing/ambiguous std owner没有fail closed；或
- production capability仍默认Unsupported；或
- Host selector仍被三处non-exhaustive match遮挡。

至少证明：

- exact linked owner round-trip到五个catch identities；
-两个不同linked caller不能交叉使用owner；
-missing/ambiguous/wrong-kind owner在发送前失败；
-local encode、deadline、ancestor cancel与五分支ordinary error分流；
-Host request success/remote/cancel/deadline/disconnect与计数归零；
-HTTP/connect evaluator严格拒绝JSON-RPC-only source。

必跑：

```bash
cargo test -p skiff-runtime-native-contract
cargo test -p skiff-runtime-linked-type-plan native
cargo test -p skiff-runtime-native websocket
cargo test -p skiff-runtime-eval native
cargo test -p skiff-runtime-host connection_request --no-fail-fast
cargo check -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

若selector名称不同，先list/确认非零执行并在result记录实际命令/count。

## 停止与交付

若outbound Host capability实际需要E0尚不存在的inbound target/API，保留纯linked-owner有效提交并返回
`TASK_SCOPE_EXPANDED`；不得创建E0模块。若exact std owner无法从current linked executable唯一取得，
返回`TASK_NOT_EXECUTABLE`并列出缺失canonical owner。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440q-rpc-invocation-linkage`
- branch：`codex/p5-f440q-rpc-invocation-linkage`
- result：`P5-F440Q-websocket-rpc-invocation-linkage-result.md`

Implementation与result分开提交；不merge/rebase/push，不运行live/stable/instance。
