# P5-F45E：I02 Canonical Spawn Submit Probe

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§7、§10、§12及§14；执行完成态来自
`P5-I02-skiff-combined-probe.md`。

DAG节点F45E，依赖F45A/F45C/F45D合流。目标是在I02 normal source→RuntimeAssembly真实请求中执行一次canonical
spawn submit control，业务可观察结果必须依赖typed Router response；不等待或声称后台claim执行，D46保持暂停。

独占写入：

- I02专用normal source fixture/API的最小扩展；
- `scripts/lib/package-service-i02-combined*.mjs`及其direct tests；
- 为authoring fixture选择/receipt ledger所需的test-runner fixture-only表面。

要求：

- 使用语言/runtime既有canonical spawn submit语义，不得新增actor legacy consumer、manual emitter、protocol peer或
  `runtime.register`；
- submit frame由F45C current ActivationContext填充完整identity，并经F45D exact assembly授权；
- typed response的稳定字段进入unary业务结果/ledger，使未执行或typed error时正例失败；
- transaction rollback、artifact-root withdrawal与R05 lifecycle证据保持原行为；
- 若normal source当前没有可表达的spawn submit语义，立即`TASK_NOT_EXECUTABLE`并给出最小compiler/runtime owner，
  不得退回`std.actor`冻结legacy surface。

开发owner运行：

```bash
node --check \
  scripts/lib/package-service-i02-combined-real.mjs \
  scripts/lib/package-service-i02-combined-oracle.mjs
node --test scripts/tests/package-service-i02-combined.test.mjs
git diff --check
```

禁止运行真实I02/R05、修改shared wire/Router/Runtime production、实现D46 worker、instance/stable/full gate。独立
worktree/branch，5分钟内修改，否则`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
