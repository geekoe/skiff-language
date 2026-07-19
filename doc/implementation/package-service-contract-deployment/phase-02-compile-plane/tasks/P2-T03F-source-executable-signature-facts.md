# P2-T03F：Source Executable Signature Facts

## 目标

把exact contract-aware type facts从“public callable专用结果”提升为全部source executable的唯一签名事实，
让public ABI view与File IR execution projection共享一个source owner。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T03B/T03C与用户选择的opaque execution representation方案A。
- 独占`compiler/source`中的contract type projection、all-function/impl-method executable signature facts、
  public callable view与直接测试。
- 不修改syntax、lowering、compiled/projection-input/projection或integration fixtures。

## 完成态

1. 新增canonical `SourceExecutableSignatureFacts`（名称可等价），覆盖所有function和impl method，按稳定source
   executable key保存exact parameters、return、receiver shape与`may_suspend`；contract leaf保留
   `ContractTypeId`，container/nullable递归保真。
2. `SourceCallableSignatureFacts`只从该事实表结合public binding产生view，不再次解析AST/type text；普通function
   与public-instance operation覆盖完整且receiver trimming只有一个owner。
3. contract-aware type projection只有一个实现；`package_type_contains_contract`等规则不重复。
   `ResolvedTypeRef.source_text`解析失败不能静默fallback到File IR/local projection。
4. unsupported inline contract shape与无法形成exact fact的executable fail closed；不存在seed/unassigned fallback。

## 聚焦验收

- source tests覆盖private helper、public function、impl method/public instance、Local/Contract/nested
  container+nullable、receiver与`may_suspend`。
- 负例覆盖missing/duplicate fact、inline shape与禁止的source-text fallback。
- 运行source聚焦测试/check、changed-file rustfmt和`git diff --check`，不运行Phase gate。

## 执行合同

- DAG：波次9a source canonical checkpoint；可与T03E按文件ownership并行，但不得修改
  `contract_call_typing.rs`、其checker拆分或`expression_type_model.rs` projected environment；完成后解除
  T03G/T04B。风险：高；
  进入typed-contract production复验组。
- worktree：`/Users/geek/workspace/skiff-p2-t03f-source-exec-signatures`；分支：
  `codex/p2-t03f-source-exec-signatures`；从调度时integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试或重做设计。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；source type resolution、executable key/public
  binding或contract schema变化即失效。
