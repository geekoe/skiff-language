# P5-F440W0 Pinned RPC execution route context result

状态：`PASS / PINNED_ROUTE_CONTEXT_VALID`。

本 leaf 在 generation pin owner 内新增 Host-private
`ResolvedWebSocketJsonRpcExecution { target, method_route }`，一次 exact join 同时返回
generation-pinned `RuntimeAssemblyWebSocketJsonRpcTarget` 与同源 old
`ActiveAssemblyRoute`。现有 `websocket_jsonrpc_target(...)` 只委托新 resolver 并投影
`target`，没有扩大 runtime/request public target。

## 1. 基线与提交

| 状态 | Commit | Tree |
| --- | --- | --- |
| 任务 implementation baseline | `2abd8c9e` | — |
| worktree 实际起点 | `d0f86d5ab3371315f584776a0113eb4d2561a68b` | — |
| implementation | `993ca5cb9fa69feaa5ec8dab605ee9ff0121bfe3` | `bfbfd85ee7ff643d5617476fa66e1f8c73fcc5ae` |

`d0f86d5a` 相对 `2abd8c9e` 只增加本任务声明，没有 production/test 变化。

Worktree：
`/Users/geek/workspace/skiff-p5-f440w0-pinned-route-context`

Branch：
`codex/p5-f440w0-pinned-route-context`

Implementation 与本文 result 分离提交；result commit 由最终交付消息记录。

## 2. Test-first compile RED

先把 existing A/B generation test 切到尚不存在的
`websocket_jsonrpc_execution_route(...)` 并读取 `.target`：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host websocket_jsonrpc_execution_route --no-fail-fast
```

结果为 exit `101`。编译器在两个 A/B 调用点精确报告：

```text
error[E0599]: no method named `websocket_jsonrpc_execution_route` found
help: there is a method `websocket_jsonrpc_target` with a similar name
```

该 RED 命中新 direct test 源码，失败来自 generation resolver API 缺失；不是零测试、依赖下载、
live 环境或既有断言失败。

## 3. Host-private exact resolver

generation registry 现在按以下单一路径解析：

```text
exact acquired (session, connection) pin
  -> saved old physical ActiveAssemblyRoute
  -> old assembly immutable sibling method ActiveAssemblyRoute
  -> RuntimeAssemblyWebSocketJsonRpcTarget
  -> { target, method_route }
```

新 owner 与方法均为 `host::websocket_generation` 私有面；返回的 method route 继续持有原有：

- old `Arc<ActiveAssembly>` / `ActiveAssemblyContextSet`；
- activation 与 execution image owner；
- deployment owner、implementation package build 与 deployment policy；
- exact `DbCapabilitySource`；
- exact `ServiceProtocolIdentity`；
- method/physical selector、key、identity、profile 与 adapter/handler join。

resolver 没有读取 current assembly pointer，没有重新打开 artifact，也没有从 target 字段重建
DB/protocol context。`websocket_jsonrpc_target(...)` 调用相同 resolver 后只移动 `.target`，因此
F440U 的 API 与调用语义保持不变。

## 4. 可区分 A/B direct fixture

generation lifecycle 现有 fixture 机械扩展出两份同 host/path/method 的 immutable assembly：

- A/B 使用不同 service/deployment、service protocol identity、deployment policy、activation、
  execution image 与 implementation package build；
- 两份 fixture 都带真实 database state metadata/binding；
- non-live test DB provider 每次 build 返回行为可观测的 generation marker source，不创建真实 DB
  store，也不访问 MongoDB；
- A 先 committed/pinned，B 再成为 current active，故测试可精确识别 `target A + context B` 混合。

direct assertions 证明：

- old connection 的 target 与 method route 都是 A；
- replacement 后的新 connection 两者都是 B；
- route/target activation 与 execution image `Arc` 指针一致；
- returned route 的 DB marker、protocol identity、policy、deployment/build owner均与同代 route
  相同且不同于另一代；
- physical selector/key/identity/id 与 method selector/key/identity/profile join保持 exact；
- target-only API 与新 resolver `.target` 的 activation/image指针及所有公开 target facts逐项等价。

## 5. Fail-closed lifecycle与tuple

direct resolver tests覆盖并拒绝：

- wrong Router session 与 connection；
- wrong assembly identity/generation；
- wrong physical `WebSocketEntryId`；
- wrong host/path/method/method gateway identity；
- tentative pin / 无 exact acquire ACK；
- release 后的 lookup；
- session disconnect 后的 lookup。

release 与 disconnect 测试均先把 A 变成 retired generation，再证明最后的 route/pin owner释放后
old `ActiveAssemblyContextSet` 可回收。

`GatewayWebSocketRpcProfile` 当前是只有
`JsonRpc2_0Text` 的 closed typed enum，Rust direct caller不存在第二个合法 profile值；测试断言
非 canonical profile在 typed deserialize边界被拒绝，并同时断言 returned target保持 exact
`JsonRpc2_0Text`。没有用 invalid-discriminant `unsafe` 伪造不存在的 enum值。

## 6. 规定 GREEN

所有 Cargo 命令统一使用：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

| 命令 | 实际执行 | 结果 |
| --- | ---: | --- |
| `cargo test -p skiff-runtime-host websocket_jsonrpc_execution_route --no-fail-fast` | 4 | PASS：4 passed / 296 filtered；3 integration binaries各 0 executed |
| `cargo test -p skiff-runtime-host websocket_jsonrpc_target --no-fail-fast` | 6 | PASS：6 passed / 294 filtered；3 integration binaries各 0 executed |
| `cargo test -p skiff-runtime-host websocket_generation --no-fail-fast` | 10 | PASS：10 passed / 290 filtered；3 integration binaries各 0 executed |
| `cargo check -p skiff-runtime-host` | — | PASS |
| `cargo fmt --all -- --check` | — | PASS |
| `git diff --check` | — | PASS |

Cargo 只输出 repository 既有 advisory unused/dead-code/unreachable-pattern warnings，没有 test/check
失败。没有运行 live/server/stable instance/watch/chat smoke。

## 7. Source / reverse audit

colocated source audit提取
`acquired_physical_route -> websocket_jsonrpc_execution_route -> websocket_jsonrpc_target`
resolver 段，并验证 exact join 顺序；以下 token均不存在：

```text
lookup_active_assembly
assembly_admission
resolve_runtime_assembly
artifact_store
FilesystemRuntimeAssembly
```

Implementation commit只修改三份任务允许文件：

- `runtime/host/src/host/websocket_generation.rs`
- `runtime/host/src/host/router_session/tests/websocket_generation_lifecycle.rs`
- generation lifecycle 已使用的
  `runtime/host/src/host/router_session/tests/runtime_assembly_request/fixture.rs`

没有修改 loader production、runtime/request target、eval、Host request entry、transport/wire、
Router、artifact/compiler 或其它 task/result。没有派子 Agent；没有 merge、rebase或 push。

## 8. 自验收矩阵

| 任务条款 | 代码证据 | 测试证据 |
| --- | --- | --- |
| target + old method route一次返回 | `ResolvedWebSocketJsonRpcExecution` 与单次 old sibling join | A/B resolved owner direct test |
| target-only API保持 | delegate后只投影 `.target` | 所有 public target facts逐项等价 |
| A不混入current B context | pin保存 old physical route，method从old immutable table解析 | DB/protocol/policy/activation/image/deployment/build A/B区分 |
| exact tuple fail closed | acquired pin tuple + sibling exact join | wrong session/connection/generation/id/host/path/method/identity |
| tentative/release/disconnect不暴露 | acquired flag与现有 lifecycle owner | tentative、release、disconnect direct tests及old owner回收 |
| 无current/artifact lookup | resolver只调用 pin route sibling/target方法 | colocated source audit + reverse search |
| 不泄漏public target | Host-private resolved owner持有 `ActiveAssemblyRoute` | production diff无 runtime/request 改动 |
