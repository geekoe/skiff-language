# P5-F429A Runtime/Host current WebSocket connect execution

状态：Ready。高风险 runtime execution与capability checkpoint。

## 直接父节点

- `P5-F429-connect-execution-consumer-wave.md`

父节点已汇总F426A wire、F425A authoring/deployment和F424A consumer audit。启动时只读本任务；
实现需要时再沿父节点引用向上查阅。

## DAG 位置

与F429B Router并行。完成后必须与F429B合流，才解除D4 fixture/tooling convergence和current
connect/downlink combined probe。输入为父节点冻结的
`1f52b2f5053830134e59bfa6f5c67d787078efa2`；当前不是稳定候选。

## 写入范围

只允许以下Rust owner及直接tests：

- `artifact-model/src/websocket_ingress.rs`、其直接tests与仅为删除该compatibility API所需的
  `artifact-model/src/lib.rs`
- `runtime/activation/src/context.rs`
- `runtime/loader/src/runtime_assembly/**`
- `runtime/linker/src/assembly/gateway.rs`
- `runtime/host/src/loader/{active_assembly_context.rs,assembly_admission.rs}`
- `runtime/host/src/host/request_entry/**`
- `runtime/boundary/**` 与 `runtime/linked-type-plan/**` 中F425A result第5节列出的旧
  receive/context shape owner
- `runtime/request/**`、`runtime/request-contract/**`
- `runtime/eval/**` 中旧WebSocket contract plan/identity/receive owner和current connect执行接线
- `runtime/host/src/{capability_context,eval_capability_adapter}/**` 中WebSocket接线
- `runtime/native/**`、`runtime/native-contract/**` 的targeted tests
- Rust generation lifecycle中connect admission/release所需owner
- `runtime/transport/src/{ingress_selector,protocol,request_mapper,response_mapper}.rs`中仅旧
  receive/context consumer及直接tests
- 本leaf result

禁止修改F426A的`runtime_assembly_request*` current wire/corpus、Router、compiler/authoring/
deployment producer、test-runner、Internals或skiff-packages。若需要改变wire、gateway policy或
公共std签名，返回`TASK_SCOPE_EXPANDED`。

## 必须实现

1. assembly admission从exact linked `ServiceDeployment`解析零或一个WebSocket entry，验证
   selector、entry key、gateway identity、surface与canonical `WebSocketEntryId` exact join。
   多entry、dangling key、identity/surface/id mismatch必须在admission fail closed。
2. `ActivationContext`保留typed optional sole-entry record。零entry合法，但四个WebSocket send
   native返回unavailable；不得用空字符串、默认entry或caller参数补齐。
3. current `websocketConnect` request按F426A closed union进入Host：header pinned entry必须与
   activation record exact match；有handler时只按F425A adapter plan调用exact private callable，
   source仅connect request/connection id。
4. 将non-generic `WebSocketConnectResult`精确映射为F426A accept/reject wire；没有Context、
   payload、receive/message或service operation lookup。
5. 无handler entry不进入Runtime/Host；Router后继负责synthesized accept。Runtime收到无handler
   dispatch必须fail closed。
6. HTTP、ordinary service call、actor和connect activation调用四个native时，都从activation
   sole-entry生成capability；四个public签名和`may_suspend=false`保持不变。
7. outbound frame继续携带service + entry；empty target、stale/mismatched generation和缺失sender
   fail closed。不得把version/build加入business fan-out key。
8. 删除F425A result第5节D2 allowlist中的旧receive/message/Context/operation consumer和不可达
   compatibility shape；current production路径反搜只能保留显式negative fixture或历史文档。

## 关键入口与遮挡

真实链：

```text
RuntimeAssembly admission
  -> ActivationContext sole entry
  -> current websocketConnect Host request
  -> exact callable + accept/reject response

HTTP/service/actor activation
  -> WebSocket native capability
  -> connection.send control frame
```

本leaf不拥有Router socket/gateway，因而只能证明response/control frame出口；client可观察发送和
1003行为由F429B及合流probe证明。上游admission失败会遮挡connect执行与native tests，测试应分别构造
通过admission的exact正例和各join负例。

## 验证

本Agent是以下聚焦验证的唯一owner：

```bash
cargo test -p skiff-artifact-model \
  -p skiff-runtime-activation -p skiff-runtime-loader \
  -p skiff-runtime-linker -p skiff-runtime-request \
  -p skiff-runtime-request-contract -p skiff-runtime-transport \
  -p skiff-runtime-eval -p skiff-runtime-native \
  -p skiff-runtime-native-contract -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

若单条combined命令受D4 fixture compile seam遮挡，必须按package运行所有实际可执行的direct suites，
记录精确discovery和遮挡，不能修改test-runner或伪报PASS。最早风险探针至少覆盖：sole-entry
admission正例、>1/dangling/mismatch负例、connect accept/reject、无handler dispatch拒绝、普通HTTP
activation调用native带exact entry、零entry unavailable。

代码、schema、Rust current wire、generation owner或相关fixture变化会使证据失效；F429B
Router-only改动不使本leaf聚焦证据失效。

## Worktree、提交与交付

- worktree：`/Users/geek/workspace/skiff-p5-f429a-runtime-connect`
- 分支：`codex/p5-f429a-runtime-connect`

启动后5分钟内完成第一次实际代码修改；否则返回`TASK_NOT_EXECUTABLE`。提交implementation，再新增
并提交`P5-F429A-runtime-websocket-connect-execution-result.md`。返回commit/tree、自验收矩阵和
clean状态。不得merge、rebase、push、stable/live；完成后不得自行承接D4或combined probe。
