# P5-F45C：Runtime Actor Activation Consumer

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§7、§10、§12及§14。

DAG节点F45C，依赖F45B，与F45D并行。目标是让真实canonical Runtime从当前ActivationContext为所有actor/spawn control
填充F45B完整ActivationIdentity并消费typed response；不修改shared wire或Router。

独占写入Runtime consumer表面：

- `runtime/eval` actor/spawn request构造；
- `runtime/host` current activation context、capability adapter/actor consumer及聚焦tests；
- 为structured identity投影所需的`runtime/activation`只读转换/helper；不得改变identity语义；
- 必要的Runtime request/native consumer机械接线。

要求：

- identity只能来自当前pinned ActivationContext，完整包含assembly/generation/runtime replica/deployment；
- async continuation/callback/spawn source保留同一owner，不读取ambient global、serviceId或package build推断；
- actor put/find/remove及spawn submit/claim/renew/complete/fail全部填充；
- typed Router error/response correlation保持fail closed；
- missing current context必须在发送frame前失败，不能产生legacy frame。

开发owner运行：

```bash
cargo test --locked -p skiff-runtime-capability-context -p skiff-runtime-transport
cargo test --locked -p skiff-runtime-host host::router_session::tests:: -- --test-threads=1
cargo check --locked -p skiff-runtime-host
git diff --check
```

禁止修改F45B DTO/codec/corpus、Router、scripts/I02、fixtures或公共设计；禁止真实probe/instance/stable/full gate。
独立worktree/branch，5分钟内修改，否则`TASK_NOT_EXECUTABLE`。提交并返回自验收矩阵，不push、不merge main。
