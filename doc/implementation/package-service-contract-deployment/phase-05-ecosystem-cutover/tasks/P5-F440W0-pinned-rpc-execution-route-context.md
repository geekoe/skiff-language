# P5-F440W0 Pinned RPC execution route context

状态：Ready。F440W的最小上游；让generation resolver同时返回target与同源old method route。

## 直接父节点

- `P5-F440W-websocket-rpc-host-dispatch-result.md`
- `P5-F440U-websocket-rpc-pinned-method-target-result.md`
- `P5-F440V-websocket-rpc-typed-evaluation-result.md`

F440W证明Host构造完整capability context还需要old method route拥有的exact
`DbCapabilitySource`与`ServiceProtocolIdentity`。current assembly lookup会混合A target/B context，
因此必须由generation pin owner暴露同一resolver已经取得的old route。

实现基线为`2abd8c9e`对应的current integration tree。

## 目标与API

增加Host-private resolved owner（名称可按current conventions调整）：

```rust
struct ResolvedWebSocketJsonRpcExecution {
    target: RuntimeAssemblyWebSocketJsonRpcTarget,
    method_route: ActiveAssemblyRoute,
}

WebSocketGenerationRegistry::websocket_jsonrpc_execution_route(...)
    -> Result<ResolvedWebSocketJsonRpcExecution, ...>
```

实现必须复用一次exact join：

```text
acquired pin
  -> old physical ActiveAssemblyRoute
  -> old sibling method ActiveAssemblyRoute
  -> RuntimeAssemblyWebSocketJsonRpcTarget
```

现有`websocket_jsonrpc_target(...)`委托新resolver并只投影target，保持F440U调用方/API/test语义。

## 唯一写集

- `runtime/host/src/host/websocket_generation.rs`
- 该模块及
  `runtime/host/src/host/router_session/tests/websocket_generation_lifecycle.rs`
  中本resolver的direct fixtures/tests
- direct fixture若分文件，只允许generation lifecycle现有fixture的机械扩展
- 本leaf result

禁止修改runtime/request target、loader/admission、eval/Host request entry、transport/wire、Router、
artifact/compiler、其它task/result。不得派子Agent，不得运行live/server。

## Owner与不变量

returned `target`与`method_route`必须来自同一个old:

- `Arc<ActiveAssembly>`；
- activation owner；
- execution image；
- deployment owner；
- implementation package build；
- host/path/method、physical/method identity与profile join。

`method_route`必须保留Host后继可读取的exact：

- `DbCapabilitySource`；
- `ServiceProtocolIdentity`；
- route-owned deployment policy/bindings；
- pinned eval target/context owner。

禁止：

- 查询assembly controller/current snapshot；
- 重新从artifact store/selector解析route；
- 根据target字段重建假的DB/protocol context；
- tentative/no-receipt pin暴露任一owner；
- release/disconnect后继续返回stale route；
- 把loader/Host route塞入runtime/request public target。

该API是Host-private sibling，target的runtime/request边界保持不变。

## Test-first与验证

先加A/B context断言，使旧target-only API缺少route而compile RED，再实现。至少覆盖：

- pin A、active replacement B后，resolved target和method route均来自A；
- B新connection返回B；
- A/B用可区分DB source、service protocol identity、policy/activation，断言不混合；
- target/route activation与execution image owner pointer一致；
- wrong session/connection/generation/physical id/host/path/method/method identity/profile拒绝；
- tentative/no receipt、release、disconnect后拒绝并可回收；
- existing `websocket_jsonrpc_target`结果与new resolver `.target`精确等价；
- source/reverse audit证明无current assembly/artifact lookup。

必跑：

```bash
cargo test -p skiff-runtime-host websocket_jsonrpc_execution_route --no-fail-fast
cargo test -p skiff-runtime-host websocket_jsonrpc_target --no-fail-fast
cargo test -p skiff-runtime-host websocket_generation --no-fail-fast
cargo check -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

先确认selectors非零并记录count。Cargo统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

## 停止与交付

若method route当前不持有完整DB/protocol context而需修改loader，返回精确缺字段与
`TASK_SCOPE_EXPANDED`；不得从current assembly补。若返回route要求扩大runtime/request public target，
停止而非泄漏Host owner。

交付：

- worktree：`/Users/geek/workspace/skiff-p5-f440w0-pinned-route-context`
- branch：`codex/p5-f440w0-pinned-route-context`
- result：`P5-F440W0-pinned-rpc-execution-route-context-result.md`

Implementation与result分开提交；不merge/rebase/push。
