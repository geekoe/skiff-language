# P5-F45A：I02 Transaction Harness

权威设计为
`doc/architecture/package-service-contract-deployment.md` §1–§15；执行完成态来自`P5-I02-skiff-combined-probe.md`。

DAG节点F45A，依赖D44 COMPLETE，可与D45设计决策并行。它只负责I02真实transaction/rollback入口，不实现或模拟
actor/spawn，不作I02/R02 verdict；完成后等待F45E/I35。

独占写入：

- 新增`scripts/lib/package-service-i02-combined*.mjs`、`scripts/tests/package-service-i02-combined.test.mjs`及必要
  I02专用oracle/helper；
- `scripts/run-package-service-ecosystem-smoke.mjs`只按`--probe skiff-cutover`委托新I02 owner，其他probe行为不变；
- 可复用现有authoring/activation/unary/isolated runtime helper，但禁止修改generation lifecycle runner、A/B fixtures、
  Router、Runtime或shared wire。

固定one-replica流程：

1. normal source author/store，valid activation并记录activationId/generation/assembly/exact replica/capability；
2. production unary返回typed业务结果；
3. 原子移走isolated canonical artifact root，重复unary仍成功，然后精确恢复；
4. 篡改同一assembly closure中的transitive production PackageArtifact record，不patch/re-sign candidate；
5. 以`expectedGeneration:1`激活同一assembly，必须收到typed `load` reject并走真实abort；
6. 断言committed tuple、旧unary result、同runtime capability/registeredAt/connected不变，pending activation为空；typed
   load reject发生在`stage_prepared`前，作为staged未分配证据；
7. 再次移走artifact root并重复旧unary成功，恢复后clean shutdown；
8. 输出exact commit/tree/lock、activationId/generation/assembly/replica、正例与rollback ledger。

所有root移动必须以isolated workspace ownership marker与精确路径校验，finally恢复；deadline覆盖authoring到cleanup。
不得用fake registry/protocol peer/manual emitter、legacy selector、retry/fallback或stable。不得删除用户/共享Cargo target；
只允许清理本任务创建且marker匹配的`/tmp/skiff-p5-i02-cargo.*`。

开发owner只运行：

```bash
node --check \
  scripts/run-package-service-ecosystem-smoke.mjs \
  scripts/lib/package-service-i02-combined-real.mjs \
  scripts/lib/package-service-i02-combined-oracle.mjs
node --test scripts/tests/package-service-i02-combined.test.mjs
git diff --check
```

禁止运行真实I02、R05、instance/stable或完整gate。独立worktree/branch从当前integration checkpoint创建，5分钟内开始
修改，否则返回`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
