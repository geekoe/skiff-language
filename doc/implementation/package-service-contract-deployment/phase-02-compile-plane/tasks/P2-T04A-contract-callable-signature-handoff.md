# P2-T04A：Contract-aware Callable Signature Handoff

## 目标

把T03B产生的exact source callable signature沿唯一
`CompiledPackage -> ProjectionInput -> PackageArtifact`路径传递，删除projection把所有类型重建为
`PackageTypeRef::Local`的blanket producer。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Package编译”章节。

## 依赖与写域

- 依赖P2-T03B；可与P2-T03C并行。
- 独占`compiler/compiled/**`、`compiler/projection-input/**`、
  `compiler/projection/src/package_artifact/**`和必要的窄driver handoff。
- 不修改source语义、lowering或integration fixtures。

## 完成态

1. source symbol到lowered executable/public path的映射在compiled owner完成一次；projection-input按现有
   executable key携带精确`PackageCallableSignature`。
2. projection对每个API callable/public-instance operation要求且只接受一个signature entry；missing、duplicate、
   extra或target不匹配fail closed。
3. contract nominal、container与nullable原样进入PackageLocalAbi callable signature；local helper仍为Local。
4. 删除`projection_signatures`中从`ExecutableSignatureIr`把参数/返回一律包成Local的producer；不得留下
   seed/unassigned fallback成为第二owner。
5. `may_suspend`与receiver trimming使用canonical lowered/source事实，public path与executable复用既有key，
   不新增aggregate adapter。

## 聚焦验收

- compiled/projection-input/projection直接测试覆盖Local、Contract、nested container/nullable、missing/duplicate
  signature和public-instance operation。
- 反向测试证明blanket Local producer不能恢复。
- 运行涉及crate的最小检查与`git diff --check`，不运行Phase gate。

## 禁止项

- 不从File IR/display string/ServiceSymbol重建contract type，不新增Publication/Release aggregate。

## 执行合同

- DAG：波次8c、与T03C并行；完成后与T03C/T03D/R10H共同解除R10I。风险：高；typed-contract production
  独立验收组。
- worktree：`/Users/geek/workspace/skiff-p2-t04a-contract-signatures`；分支：
  `codex/p2-t04a-contract-signatures`；从含T03A/T03B的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；source signature facts、compiled/projection-input
  handoff或PackageArtifact projection变化即失效。
