# P2-R10H：Typed Contract Fixture Checkpoint

## 目标

给compiler integration tests建立最小programmatic contract dependency入口，使测试能向package compile提供
非空validated `ServiceContract`，但不发明YAML/IDL authoring，也不恢复service publication harness。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“ServiceContract”“Package编译”与
“明确不冻结的表面”章节。

## 依赖与写域

- 依赖当前canonical compiler fixture；可与P2-T03A并行。
- 独占`compiler/tests/common/**`、一个representative probe target及必要`compiler/Cargo.toml`测试声明。
- 不修改production Rust。

## 完成态

1. common fixture提供从`ServiceContract`派生`PackageContractCompileDependency`的窄helper；service coordinate、
   version和protocol identity只能来自contract本体，测试只显式给alias。
2. package graph可按package coordinate携带contract dependency slice；原无contract helper委托空slice，避免
   两套compile pipeline。
3. representative probe用非空contract dependency编译一个暂不引用contract symbol的package，证明input lane
   可达；不伪造empty contract、provider package或deployment。
4. common API保持小而明确，不新增万能builder、磁盘service config或runtime holder。

## 聚焦验收

- 运行新增probe和common受影响测试，`git diff --check`。
- production source diff应为零。

## 执行合同

- DAG：波次8a、与T03A并行；完成后等待T03C/T03D/T04A并共同解除R10I。风险：中；由R10I和阶段gate
  覆盖，不单开重复验收。
- worktree：`/Users/geek/workspace/skiff-p2-r10h-typed-fixture`；分支：
  `codex/p2-r10h-typed-fixture`；从调度时integration HEAD创建，禁止复用旧worktree。
- 启动后5分钟内完成第一次实际测试代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；common package graph/contract fixture API或
  compile-input contract shape变化即失效。
