# P5-F45D：Router Actor Activation Consumer

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§10、§12及§14。

DAG节点F45D，依赖F45B，与F45C并行。目标是让Router只从发送者exact assembly registration及active/draining
generation snapshot授权structured actor/spawn ActivationIdentity，并关闭F45B留下的旧string consumer断链。

独占写入Router consumer表面：

- `router/src/router/actorSpawnRuntimeControl.ts`、`runtimeRegistry.ts`、`runtimeEndpoint.ts`；
- assembly registration/active-draining snapshot的最小read-only查询owner；
- queue types/store中structured ActivationIdentity的必要持久表示；
- 对应actor-spawn、assembly endpoint/registry/queue聚焦tests。

要求：

- sender必须是exact canonical assembly connection；legacy `runtime.register`/package-test路径不得冒充canonical授权；
- 完整tuple必须匹配该connection registration及active或仍被pin的draining generation；drain完成、sender mismatch、
  missing/partial/legacy identity均typed fail closed；
- 不能按serviceId、buildId、display string或queue stored string补事实；
- response按原request+sender correlation；持久spawn item保留完整structured identity；
- 保留现有明确package-test路径，但它不能接受或生成canonical assembly identity。

开发owner运行：

```bash
pnpm --filter @skiff/router exec vitest run \
  tests/protocol.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/actor-spawn-runtime-control.test.ts
pnpm --filter @skiff/router type-check
git diff --check
```

必须覆盖active允许、pinned draining允许、drained拒绝、wrong replica/deployment/generation/assembly拒绝、legacy sender拒绝、
queue roundtrip及F45B 12个type errors关闭。

禁止修改F45B shared DTO/codec/corpus、Runtime、scripts/I02、fixtures或公共设计；禁止真实probe/instance/stable/full gate。
独立worktree/branch，5分钟内修改，否则`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
