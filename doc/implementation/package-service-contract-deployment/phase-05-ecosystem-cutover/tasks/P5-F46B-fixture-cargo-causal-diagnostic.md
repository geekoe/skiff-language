# P5-F46B：Fixture Cargo Causal Diagnostic

DAG节点F46B，依赖I02A exact FAIL。目标是在F26A既有最多3条、脱敏、hash/bytes/omitted count边界内，优先保留
Cargo terminal causal error，而不是被前置warning占满；不修复未知fixture/compiler根因。

独占写入：

- `scripts/lib/package-service-ecosystem-smoke-diagnostic.mjs`
- `scripts/tests/package-service-ecosystem-smoke-diagnostic.test.mjs`
- I02调用diagnostic所需的最小test fixture。

要求：

- deterministic选择最多3条，优先真实`error:`/`Caused by:`/process terminal summary及必要相邻context；
- warning-only输出仍有代表性，error在长warning前缀/后缀后都必须可检出；
- 保留candidate label、exit/signal、stdout/stderr bytes+SHA、总非空行与omitted count；
- 所有retained文本继续脱敏、单条/总长bounded，不能输出完整Cargo log；
- 不修改Cargo命令、compiler/fixture/Router/Runtime或业务行为。

开发owner运行：

```bash
node --check scripts/lib/package-service-ecosystem-smoke-diagnostic.mjs
node --test \
  scripts/tests/package-service-ecosystem-smoke-diagnostic.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs
git diff --check
```

禁止真实I02/fixture Cargo、instance/stable/full gate。独立worktree/branch，5分钟内修改，否则
`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
