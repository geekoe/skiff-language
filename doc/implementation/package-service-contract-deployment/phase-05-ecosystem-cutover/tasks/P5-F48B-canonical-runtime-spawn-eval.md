# P5-F48B：Canonical Runtime Spawn Eval

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第8、10、11条，§7、§10、§12及§14。

DAG节点F48B，依赖D48，与F48A/F48C并行。独占`runtime/eval` spawn/projection及聚焦tests。

`submit_spawn_statement`在canonical RuntimeAssembly必须消费`execution_projection()`与admitted in-memory
`AssemblyExecutionImage`，按compiler-owned spawnSubmit metadata及exact linked executable address/symbol构造
`function:` target；保留完整ActivationIdentity。legacy RuntimeProgram分支保持原行为。禁止构造legacy ServiceUnit/route、
artifact I/O、worker route或manual fallback。

direct必须证明canonical payload到Actor capability、target精确、无legacy projection/error/artifact I/O，并覆盖missing
projection/metadata fail closed。运行focused `skiff-runtime-eval` test、cargo check与diff check；禁止修改host/Router/fixture/
shared wire，禁止真实I02/R05/instance/stable。独立worktree，5分钟内修改，提交自验收矩阵。
