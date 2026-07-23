# P5-I36：Test-runner Health Combined

DAG节点I36，依赖F46A合流。全新只读owner在exact合流candidate各运行一次：

```bash
cargo test --locked -p skiff-test-runner runtime_execution::wire
pnpm --filter @skiff/router exec vitest run tests/assembly-runtime-endpoint.test.ts
pnpm --filter @skiff/router type-check
git diff --check
```

必须结构对照Router producer与Rust consumer字段名、required/non-negative integer及unknown-field策略；不运行真实fixture、
R05/I02、instance/stable/full gate，不编辑或修复。PASS只解除I35C一次真实fixture重验。
