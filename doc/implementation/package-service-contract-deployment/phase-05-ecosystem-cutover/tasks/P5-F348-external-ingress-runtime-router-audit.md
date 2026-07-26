# P5-F348 External ingress runtime / Router audit

状态：Ready（只读）。

## 直接父节点

- `P5-H35-external-ingress-surface-separation.md`

## 目标

只读追踪当前`ServiceDeployment/RuntimeAssembly -> loader/linker/activation -> Host request ->
Router HTTP/WebSocket gateway`全链，区分已有`gatewayEntryIdentity`链与错误的
`ContractOperationId` ingress链，给出唯一runtime/router收敛点。

必须回答：

1. Rust deployment/runtime assembly/transport/host哪些DTO与查表仍要求contract operation。
2. Router manifest、identity、runtime registration和gateway dispatch现有
   `gatewayEntryIdentity`由谁计算、校验、传输，Rust侧能否直接复用相同canonical owner。
3. Handler target、adapterArgs、linked signature、activation owner、timeout/cancel/error/stream在哪些
   跳点被消费；external ingress怎样进入普通request执行而不伪造service caller。
4. WebSocket connect/receive同一connection的entry identity/generation/drain状态如何保持；哪些地方仍
   绑`serviceProtocolIdentity`。
5. F346 fixed error链哪些API可复用、哪些测试会因operation identity改动失效。
6. shared model完成后可并行的loader/linker、Host/transport、Router/gateway consumers及最小真实探针。

## 范围与写入

只读检查`runtime/**`、`router/**`、`artifact-model`中runtime/deployment DTO及cross-system fixtures。
不得修改production/test/corpus/lockfile。

只允许新增：

- `P5-F348-external-ingress-runtime-router-audit-result.md`

result记录exact commit/tree、关键跳点、重复owner、首次损失、依赖方向、证据失效范围和建议DAG。
不运行workspace/stable/live，不push。提交result并返回commit。

## Worktree

- `/Users/geek/workspace/skiff-p5-f348-ingress-runtime-audit`
- `codex/p5-f348-ingress-runtime-audit`

