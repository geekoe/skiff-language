# P2-T03B：Qualified Contract Type Resolution

## 目标

让package source中的`payments.User`只通过T03A的validated contract facts解析为精确`ContractTypeId`，并产生
后续projection可直接消费的exact package callable signature事实。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“ServiceContract nominal types”与
“Package编译”章节。

## 依赖与写域

- 依赖P2-T03A。
- 独占`compiler/source`中的contract type resolution、小型新模块、相关source tests与最小facade。
- 不修改lowering、projection、compiled/projection-input或integration fixtures。

## 完成态

1. qualified type的alias命中contract dependency时，只允许解析该contract中`PublicNameable`的stable type key；
   未知、ClosureOnly、alias冲突均fail closed。
2. package dependency qualified type继续走既有规则；package/contract不靠上下文二选一。
3. source为每个可导出的callable形成精确typed signature事实：contract nominal保留`ContractTypeId`，local type保留
   local domain，builtin/container/nullable递归保真。
4. `ContractTypeRef::Record`、`StructuralUnion`、`Literal`等没有source命名与`PackageTypeRef`终态表达的inline shape
   在本阶段fail closed，不允许flatten成Local或display string。
5. 不给File IR `TypeRefIr`新增contract wire variant；若需要source内部semantic carrier，放在职责单一的小模块，
   避免继续膨胀现有数千行type-resolution文件。

## 聚焦验收

- source直接测试至少覆盖public named contract type、unknown type、ClosureOnly、nested container/nullable、
  package/contract alias冲突。
- 证明`payments.User`得到真实`ContractTypeId`，同shape的package-local type不能冒充。
- 运行source crate聚焦测试/检查和`git diff --check`，不运行Phase gate。

## 禁止项

- 不从provider source、deployment、display name或`ServiceSymbolRef`反推contract identity。
- 不复制一套与T03A不同的contract lookup/normalization规则。

## 执行合同

- DAG：波次8b关键路径；完成后解除T03C、T04A。风险：高；与T03C进入source typed-contract验收面。
- worktree：`/Users/geek/workspace/skiff-p2-t03b-contract-types`；分支：`codex/p2-t03b-contract-types`；
  从含T03A的integration HEAD创建，禁止复用旧worktree。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试或重做设计。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；T03A fact API、source type owner或contract type
  schema变化即失效。
