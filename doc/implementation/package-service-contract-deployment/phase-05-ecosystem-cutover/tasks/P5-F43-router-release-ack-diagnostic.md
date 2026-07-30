# P5-F43：Router Release ACK Diagnostic

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§6.2、§7、§12及§14。

DAG节点F43，依赖D42 COMPLETE，与F42并行；完成后与F42共同解除F44。目标是为one-replica固定顺序transcript提供
exact matching release ACK观察，排除release error、timeout或disconnect/reconnect清理造成的pin count假阳性。

独占写入：

- `router/src/router/webSocketGenerationLifecycleRouter.ts`
- `router/src/router/assemblyRuntimeRegistry.ts`
- 对应`router/tests/websocket-generation-lifecycle-router.test.ts`、
  `router/tests/assembly-runtime-endpoint.test.ts`及必要同owner health test。

按每个runtime WebSocket connection累计`connectionReleaseAckCount`：只有
`handleReleaseResponse`完成sender/request/tuple精确校验且action为ACK后递增；pending、reject、send failure、timeout、
disconnect不得递增，新连接不得继承旧连接计数。把值加入每个replica health snapshot，字段名固定为
`connectionReleaseAckCount`。不得改变release wire、pin semantics、activation、四对象或其他公共行为。

开发owner运行：

```bash
pnpm --filter @skiff/router exec vitest run \
  tests/websocket-generation-lifecycle-router.test.ts \
  tests/assembly-runtime-endpoint.test.ts
pnpm --filter @skiff/router type-check
```

direct必须覆盖pending不增、matching ACK增1、reject/timeout/send error/disconnect不增、新connection为0，并确认health
snapshot精确暴露该connection值。

禁止修改Runtime、scripts codec/harness、fixtures、compiler/store或公共wire；禁止真实transcript、instance/stable或完整
gate。独立worktree/branch，从当前integration checkpoint创建，5分钟内开始实际修改，否则返回
`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
