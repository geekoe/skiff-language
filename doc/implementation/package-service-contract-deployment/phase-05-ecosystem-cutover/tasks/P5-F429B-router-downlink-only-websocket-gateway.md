# P5-F429B Router current downlink-only WebSocket gateway

状态：Ready。高风险gateway lifecycle与sender trust checkpoint。

## 直接父节点

- `P5-F429-connect-execution-consumer-wave.md`

父节点已汇总F426A wire、F425A authoring/deployment和F424A consumer audit。启动时只读本任务；
实现需要时再沿父节点引用向上查阅。

## DAG 位置

与F429A Runtime/Host并行。完成后必须与F429A合流，才解除D4 fixture/tooling convergence和current
connect/downlink combined probe。输入为父节点冻结的
`1f52b2f5053830134e59bfa6f5c67d787078efa2`；当前不是稳定候选。

## 写入范围

只允许以下Router owner及直接tests：

- `router/src/router/{runtimeAssemblySnapshot.ts,runtimeAssemblyDeploymentSnapshot.ts,filesystemRuntimeAssemblySnapshotLoader.ts}`
- 新current assembly connect-only gateway
- `router/src/router/{server.ts,runtimeEndpoint.ts,runtimeDispatcher.ts}`
- `router/src/gateway/{webSocketGateway.ts,webSocketConnectionLifecycle.ts}`中通用lifecycle收敛
- `router/src/router/webSocketGenerationLifecycleRouter.ts`
- `router/src/index.ts`
- `router/src/manifest/**`、`router/src/artifacts/**`中的legacy WS manifest/projection residue
- `router/tests/**`，但不得修改F426A protocol corpus或protocol reader tests
- 本leaf result

禁止修改`router/src/protocol/**`、shared cross-system wire corpus、Rust runtime/compiler、
test-runner、Internals或skiff-packages。若需要改变F426A wire或F425A deployment/schema语义，返回
`TASK_SCOPE_EXPANDED`。

## 必须实现

1. current RuntimeAssembly v2 snapshot与exact ServiceDeployment snapshot接受
   `websocketConnect` entry和optional handler，并验证selector/key/gateway identity/
   `WebSocketEntryId` exact join；不得回退到legacy manifest或operation route。
2. server按current assembly WebSocket host/path binding升级连接，构造F426A exact connect request，
   pinned service/deployment/assembly/generation/entry identity在整个connect/lifetime保持一致。
3. 有handler时通过current RuntimeDispatcher发送connect request并消费exact accept/reject；
   accept应用可选business identity/policy，reject关闭且不admit。
4. 无handler时Router synthesized accept：零runtime dispatch、零runtime generation acquire，
   但连接仍保存exact snapshot/entry ownership用于policy、fan-out与outbound authorization。
5. client text或binary data到达时第一步close `1003`和有界reason；不得parse、enqueue、
   `scheduleReceive`、build request或dispatch runtime。ping/pong后连接保持；peer close正确deindex。
6. 从通用lifecycle删除receive queue、active receive、pending counters与业务receive scheduling；
   不保留“暂时不可达”的business uplink路径。
7. 有handler的generation pin覆盖整个连接；socket close、reject、error exact-once release。
   无handler不能伪造runtime pin。
8. `connection.send`真实gateway验证sender pinned assembly/generation/replica、frame serviceId和
   websocketEntryId、direct target owner；mismatch不能向client发送，并按冻结policy关闭违规runtime。
   closed direct-send race安全返回miss。
9. business fan-out只按`(service, entry, businessIdentity)`，跨version/build仍fan-out；generation
   只用于owner/pin，不进入business key。
10. 删除Router legacy connect/receive/context manifest/projection/gateway residue；F426A报告的30个
    旧gateway/receive tests必须被删除或重写为current行为，不能用dual-read/compatibility恢复通过。

## 关键入口与遮挡

真实链：

```text
HTTP upgrade -> current assembly binding -> optional runtime connect
             -> lifecycle/index/policy
runtime connection.send -> exact sender authorization -> socket downlink
client data -> immediate 1003, zero runtime dispatch
```

本leaf没有Rust Host执行，因此focused tests使用exact dispatcher seam；有handler的真实跨进程
accept/reject由F429A合流后的combined probe证明。snapshot validation失败会遮挡gateway与sender
tests，必须分别覆盖snapshot正负例和已admit lifecycle入口。

## 验证

本Agent是以下聚焦验证的唯一owner：

```bash
pnpm --dir router test
pnpm --dir router exec tsc --noEmit
node scripts/verify.mjs --only router
git diff --check
```

不能修改current protocol reader/corpus来消除失败。最早风险探针至少覆盖：有handler
accept/reject、无handler accept且0 dispatch/acquire、text/binary 1003且0 dispatch、ping/pong、
exact-once release、sender service/entry/generation mismatch、closed direct-send race和跨build
business fan-out。

Router gateway/snapshot/lifecycle/protocol wire、deployment schema或相关tests变化会使证据失效；
F429A Rust-only改动不使本leaf聚焦证据失效。

## Worktree、提交与交付

- worktree：`/Users/geek/workspace/skiff-p5-f429b-router-connect`
- 分支：`codex/p5-f429b-router-connect`

启动后5分钟内完成第一次实际代码修改；否则返回`TASK_NOT_EXECUTABLE`。提交implementation，再新增并
提交`P5-F429B-router-downlink-only-websocket-gateway-result.md`。返回commit/tree、自验收矩阵和
clean状态。不得merge、rebase、push、stable/live；完成后不得自行承接D4或combined probe。
