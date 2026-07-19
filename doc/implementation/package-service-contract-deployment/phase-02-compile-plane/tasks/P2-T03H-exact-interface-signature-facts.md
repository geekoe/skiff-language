# P2-T03H：Exact Interface Signature Facts

## 目标

让interface operation与function/impl executable一样，在source层先拥有contract-aware exact signature，完成
exact conformance，并冻结供lowering与compiled handoff共同消费的唯一query surface。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T03F及方案A。
- 独占`compiler/source`中的interface exact requirement/conformance facts、共享contract-aware type resolver、
  source model query surface与直接测试。
- 不修改lowering、compiled/projection-input、PackageArtifact projection、artifact schema或integration fixtures。

## 完成态

1. 新增canonical source-owned interface operation signature facts（名称可等价），按稳定interface symbol与method
   key保存exact parameters、return、generic/Self substitution所需事实及method flags；contract leaf保留
   `ContractTypeId`，container/nullable递归保真。
2. interface requirement与impl executable共享T03F的contract-aware type resolver/`PackageTypeRef`规则；不得
   再各自解析qualified text形成alias-shaped `ServiceSymbol`。
3. impl/interface conformance在source层、execution erasure之前比较exact facts。同一`ContractTypeId`经不同
   validated alias仍相等；不同`ContractTypeId`必须失败。missing method、arity、receiver、generic/Self或
   unsupported inline contract shape继续fail closed。
4. `PackageSourceModel`暴露职责窄且稳定的validated interface query：interface/method key、exact requirement、
   receiver conformance与canonical substitution结果；lowering和compiled后续只消费该API，不再调用旧
   `InterfaceSemantics`/`TypeResolutionModel`重算conformance。
5. 旧`InterfaceSemantics`可以继续负责不含contract identity的局部interface结构查询，但不得再成为contract
   signature/conformance owner；无法形成exact fact的interface use fail closed。

## 聚焦验收

- source tests覆盖Local/Contract/nested container+nullable、generic/Self substitution、同identity不同alias与
  不同identity负例、missing/mismatched impl method。
- 反向测试证明alias-shaped `ServiceSymbol`不能冒充exact contract conformance，query结果不受无关external
  symbol影响。
- 运行source最小测试/check、changed-file rustfmt和`git diff --check`，不运行Phase gate。

## 执行合同

- DAG：波次9d的共享source checkpoint；完成后并行解除T03I与T04C。风险：高；source/interface semantic
  checkpoint。
- worktree：`/Users/geek/workspace/skiff-p2-t03h-exact-interface-signatures`；分支：
  `codex/p2-t03h-exact-interface-signatures`；从含T03G/T04B/R10I的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试或重做宽泛设计。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；contract resolver、source interface facts、
  executable facts或validated interface query surface变化即失效。

