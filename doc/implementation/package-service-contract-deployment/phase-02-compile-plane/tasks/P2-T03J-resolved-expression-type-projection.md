# P2-T03J：Resolved Expression Exact-type Projection

## 目标

删除expression exact-type projection把`ResolvedTypeRef`的debug/display文本重新当源码解析的错误路径；让source
origin type只经canonical resolver投影一次，并以完整`PackageTypeRef` sidecar贯穿派生表达式与binding。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Compiler 与 Projection 流水线”章节；Phase 02方案A与inline contract shape fail-closed规则。

## 依赖与写域

- 依赖T03B、T03F及T07 `runtime_slots::map_keys_and_for_in_lower_to_typed_slots` finding；T07已提交canonical
  dependency-address fixture修复`e3cbffd`。
- 独占`compiler/source/src/contract_type_resolution/**`、
  `compiler/source/src/expression_type_model/contract_call_typing/**`、`expression_type_model.rs`中派生exact type
  的窄传播点及直接source测试。
- 不修改artifact model/identity、TypeRefIr schema、lowering、compiled/projection-input/projection、
  `compiler/tests/runtime_slots.rs`或其它integration fixture。

## 完成态

1. `ResolvedTypeRef.ir`是派生表达式的canonical resolved事实；`source_text`只作诊断，不再包装回AST `TypeRef`
   或重新parse。`LocalType { type_index }`精确保留为`PackageTypeRef::Local`，不是可解析的`#0`源码拼写。
2. source-origin `TypeRef`仍且只经`ContractAwareTypeResolver`形成完整
   `PackageTypeRef::{Local, Contract, Container, Nullable}`；contract leaf只凭validated alias/stable key获得
   `ContractTypeId`，不得从`ServiceSymbol`或display文本反推。
3. expression projection sidecar不再只保存“contains contract”的子集。Map keys、单binding/双binding for-in及
   generic/local call从已有container参数传播完整exact projection，local-only路径也不得丢失。
4. unresolved contract key、unsupported non-container generic与inline record/union/function contract shape继续
   fail closed；不得把未知类型默认为Local/unknown或新增AST/display fallback。
5. canonical resolver与expression sidecar各自职责单一；不得新增第二套contract identity解析或generic solver。

## 聚焦验收

- direct source tests覆盖`LocalType`、`Array<LocalType>`、Map keys、单/双binding for-in，以及container/nullable中
  contract leaf保真；未知contract/inline unsupported负例保持。
- `cargo test -p skiff-compiler --test runtime_slots map_keys_and_for_in_lower_to_typed_slots -- --exact --nocapture`。
- source/compiler最小check、changed-file rustfmt与`git diff --check`；不运行R10I/F09D/T07宽gate。

## 执行合同

- DAG：波次9k source exact projection checkpoint；完成后解除R10I/F09D证据刷新，再由T07恢复剩余gate。
  风险：高；source exact-type owner。
- worktree：`/Users/geek/workspace/skiff-p2-t03j-resolved-expression-types`；分支：
  `codex/p2-t03j-resolved-expression-types`；从`e3cbffd`创建。
- 启动后5分钟内完成第一次实际代码修改；修改前不跑测试、重做设计或扩大搜索。若完整sidecar需要改变
  `PackageTypeRef`/TypeRefIr公共shape，立即回报`TASK_NOT_EXECUTABLE`，不得自行改schema。
- 提交一个聚焦commit和自验收矩阵，不push。
