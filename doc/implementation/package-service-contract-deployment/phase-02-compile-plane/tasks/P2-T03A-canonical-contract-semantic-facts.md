# P2-T03A：Canonical Contract Semantic Facts

## 目标

把validated contract dependency从compile-input trust boundary一次性投影成source共享的typed semantic facts，
并在进入source分析前冻结package/contract共用的alias namespace。后续type resolution、call typing和lowering
只能消费这份事实，不能各自重建operation/type索引。

权威语义见`doc/architecture/package-service-contract-deployment.md`的“ServiceContract nominal types”和
“Package编译”章节。

## 依赖与写域

- 依赖当前Phase 02 terminal checkpoint与用户确认的qualified alias规则。
- 主要写域：`compiler/input/src/contract_dependencies/**`、
  `compiler/source/src/dependency_analysis.rs`、`compiler/source/src/resolved_call_targets/**`、
  `compiler/driver/source_compile/canonical_dependencies.rs`及为保持typed carrier编译所需的窄consumer。
- 不实现contract call参数检查、qualified type lowering或PackageArtifact signature projection。

## 完成态

1. 每个contract dependency在input boundary完成identity validation；source facts携带精确
   `ContractRequirement`与对应validated `ServiceContract`，可按alias/stable key查询operation descriptor和
   public-nameable `ContractTypeId`。
2. `SourceDependencyAnalysisInput`使用fallible constructor；package alias与contract alias相交、任一侧重复alias
   均在source compile开始前fail closed，不再产生`Ambiguous`后继续分析。
3. resolved contract call target携带lowering所需的完整contract requirement和真实operation identity；不复制
   protocol/display/provider/deployment事实。
4. driver不再创建新的operation-only source索引。当前lowering旧索引只允许作为T03D将删除的既存中间owner，
   不得扩展或成为新API。
5. 未知alias/member、ClosureOnly type和identity不匹配有稳定、可定位的错误；不从provider package或字符串猜测。

## 聚焦验收

- input/source dependency直接测试覆盖合法lookup、重复alias、package/contract alias冲突和未知member。
- resolved call-target serde/consumer测试随新typed shape更新。
- 运行改动crate的最小`cargo test`或`cargo check`及`git diff --check`；不运行Phase gate。

## 禁止项

- 不新增contract authoring语法、provider inference、compatibility carrier或第二份full-operation index。
- 不把contract nominal type编码进`TypeRefIr::ServiceSymbol`作为artifact事实。

## 执行合同

- DAG：波次8a共享检查点；完成后解除T03B、T03D。风险：高；进入typed-contract production独立验收组。
- worktree：`/Users/geek/workspace/skiff-p2-t03a-contract-facts`；分支：`codex/p2-t03a-contract-facts`；
  从调度时integration HEAD创建，禁止复用旧worktree。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`和最小缺口，修改前不跑测试。
- 开发Agent提交一个聚焦commit并回报自验收矩阵。证据只对该commit有效；上述写域、contract schema/
  identity或上游dependency API变化即失效。
