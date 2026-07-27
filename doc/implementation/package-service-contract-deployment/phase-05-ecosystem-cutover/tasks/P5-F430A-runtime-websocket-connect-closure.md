# P5-F430A Runtime WebSocket connect closure

状态：Ready。F429A scope expansion后的高风险completion checkpoint。

## 直接父节点

- `P5-F429A-runtime-websocket-connect-execution-result.md`

父节点记录了safe implementation checkpoint、两个新增production owner、ordinary service-call
capability缺口和未建立的测试证据，并继续引用到F429 wave、F426A wire及唯一权威设计。启动时只读
本任务；实现需要时再沿引用向上查阅。

## DAG位置与精确输入

本节点替代未完成的F429A；与F429B Router继续并行。输入为：

| commit | tree |
| --- | --- |
| `bbe5233d71dc143458371aa366dbc385c7dcd261` | `0344ee467060fb374c5b3f80f7ebe478cd734c3a` |

其中已包含F429A safe checkpoint：

- current connect admission、activation typed sole-entry record；
- exact callable execution与accept/reject current response mapper；
- HTTP/connect/actor顶层activation的WebSocket capability接线；
- `cargo check -p skiff-runtime-host`通过。

完成后与F429B合流，才解除D4 fixture/tooling和combined probe。当前不是稳定候选。

## 写入范围

恢复F429A原授权的Rust D2范围，并新增父result第6节的精确机械闭合owner：

```text
runtime/driver/eval/mod.rs
runtime/driver/eval/eval_context/tests.rs
runtime/driver/eval/tests/program_execution.rs
runtime/host/src/host/http_response_ceiling.rs
```

原D2范围限于：

- `artifact-model/src/websocket_ingress.rs`、direct tests、必要的`artifact-model/src/lib.rs`
- `runtime/{activation,boundary,linked-type-plan,request,request-contract,eval}/**`
- `runtime/loader/src/runtime_assembly/**`
- `runtime/linker/src/assembly/gateway.rs`
- `runtime/host/src/loader/**`
- `runtime/host/src/host/request_entry/**`
- `runtime/host/src/{capability_context,eval_capability_adapter}/**`
- runtime native/native-contract targeted tests、Rust generation lifecycle
- `runtime/transport/src/{ingress_selector,protocol,request_mapper,response_mapper}.rs`的旧consumer
- 上述owner的direct tests与本leaf result

禁止修改F426A `runtime_assembly_request*` current wire/corpus、Router、compiler/authoring/
deployment producer、test-runner、std、Internals或skiff-packages。新增四个owner只授权机械删除旧
re-export/exhaustive match/struct literal字段，不能新增公共语义。

## 必须完成

1. 保留并补测F429A safe checkpoint的sole-entry admission exact join、typed activation record、
   current connect handler执行、accept/reject映射、无handler Runtime拒绝和connect generation pin。
2. 删除F425A result第5节D2 allowlist内全部旧receive/message/Context/operation consumer与
   compatibility shape；不能保留alias或dual-read来绕过编译。
3. `runtime/driver/eval/mod.rs`删除失去source owner的`websocket_adapter` shim和named legacy
   re-export；仅当struct literal编译闭合需要时删除两个direct fixture的旧字段初始化。
4. `http_response_ceiling.rs`删除legacy `ResponseEnd::WebSocket` exhaustive arm与旧Context test；
   current connect response继续走专用current mapper，不进入HTTP ceiling。
5. ordinary in-process service call切换到provider activation时，重建provider自己的WebSocket
   capability，不能继承caller service/entry。覆盖：
   - caller/provider都有不同entry，control frame使用provider owner；
   - provider无entry，四个native为unavailable；
   - caller无entry而provider有entry，provider可用；
   - HTTP/connect/actor现有顶层接线不回归。
6. admission负例覆盖多entry、dangling key、selector/key/gateway identity/surface/internal entry id
   mismatch；零entry合法。
7. connect执行覆盖accept/reject、optional identity/policy、header/activation mismatch、无handler
   dispatch拒绝、空payload与stale generation；无Context/payload/receive/service operation lookup。
8. 四个native签名和`may_suspend=false`不变，outbound control继续携带service + sole entry；
   version/build不进入business fan-out key。

## 验证

本Agent是下列聚焦证据的唯一owner：

```bash
cargo test -p skiff-artifact-model \
  -p skiff-runtime-activation -p skiff-runtime-boundary \
  -p skiff-runtime-linked-type-plan -p skiff-runtime-loader \
  -p skiff-runtime-linker -p skiff-runtime-request \
  -p skiff-runtime-request-contract -p skiff-runtime-transport \
  -p skiff-runtime-eval -p skiff-runtime-native \
  -p skiff-runtime-native-contract -p skiff-runtime-host
cargo check -p skiff-runtime-driver
cargo fmt --all -- --check
git diff --check
```

若D4 test-runner seam在source execution前阻断某个package test，按package拆开所有可执行direct suite，
记录精确遮挡，不得越界修改或伪报PASS。反向搜索必须证明旧receive/context/operation production
owner归零到明确的协议历史/negative allowlist；current `websocketConnect` spelling不是legacy命中。

任何D2 production、wire、deployment schema、generation owner或相关tests变化都会使证据失效；
F429B Router-only改动不使本leaf聚焦证据失效。

## Worktree、提交与交付

- worktree：`/Users/geek/workspace/skiff-p5-f430a-runtime-connect-closure`
- 分支：`codex/p5-f430a-runtime-connect-closure`

这是新开发Agent会话，不复用F429A会话。启动后5分钟内完成第一次实际代码修改；若再次发现新的
production owner、公共契约或多路径不明确，按工作流停止并返回精确证据。提交implementation，再新增
并提交`P5-F430A-runtime-websocket-connect-closure-result.md`。返回commit/tree、自验收矩阵和clean
状态。不得merge、rebase、push、stable/live；完成后不得自行承接D4或combined probe。
