# P5-F285 Dependency result field-read fix

状态：Ready。

## 直接父节点与权威链

- 直接父结果：
  `P5-F283-dependency-result-field-read-audit-result.md`
- 诊断父结果：
  `P5-F282-dependency-result-field-read-regression-result.md`
- F282继续引用F269/F273以及唯一架构与静态语义事实源。

启动时只读本任务；需要依据时沿父链向上读取。

## DAG位置、基线与集成顺序

- 节点：F269真实Agine验收的shared compiler unblocker。
- 开发production base：`7045644f49c739510365aa33520f9da3d3f9e399`。该branch保留F283审计结果，
  production仍处于A1 strict model合入前的可编译状态。
- 当前integration已冻结F281/A1 strict model，compiler consumer有意暂时不能编译；因此本节点不从当前
  integration直接开发，也不得为通过编译恢复F281删除的field。
- 本提交只修改`compiler/source`与一个compiler integration test，完成后合入当前integration；
  F280 W2-L language consumer随后从包含本提交的checkpoint开始并保留该回归。
- F269继续等待；本节点合入且W2 consumer恢复compiler可编译后，才通知F269重跑fresh Agine。

## 已确认首次损失

`OwnerChecker::call_type`已经拿到dependency artifact的完整exact callable signature，但只要任一parameter含
`PackageTypeRef::Local` slot，就把参数和return一起filter掉。Agent的：

```text
stopThread(bindings: Local(ServiceSymbol(...))) -> PackageSchema(StopThreadResult)
markDeleted(bindings: Local(ServiceSymbol(...))) -> PackageSchema(MarkDeletedResult)
```

因此call同时失去ordinary owner-bound type与exact `PackageSchema` projection，后续local binding/member
lookup/object literal只是在传播上游`None`。

## 唯一production写入范围

- `compiler/source/src/expression_type_model.rs`
- `compiler/source/src/type_resolution_model/shape_assignability.rs`
- 仅确有必要时：
  `compiler/source/src/type_resolution_model.rs`中的窄owner-aware normalization API/错误导出

测试：

- `compiler/tests/package_imports.rs`
- 仅为隔离normalizer负例确有必要时，`compiler/source` co-located test module

禁止修改artifact model/schema/identity/projection、canonical dependency ingest、syntax/lowering/runtime、
Agent/Agine、F281 error DTO、reference/architecture或其它任务文件。

## 完成标准

1. `SourceDependencyAnalysisInput`中已验证的exact callable signature仍是唯一callable事实源；不在
   `TypeResolutionModel`复制artifact callable index。
2. 以dependency alias、package id、expected Local ABI与artifact type slots递归rehydrate signature中的
   `PackageTypeRef::Local`：
   - `LocalType`
   - `PublicationType`
   - `ServiceSymbol`
   - `PackageSymbol`
   - nested container/nullable/union/record/function/any-interface中的上述引用
3. normalization产生唯一owner-bound ordinary type；不能按短名、display string、shape、package/callable/
   field特判。`PackageSchema`的owner/key/type id保持原样。
4. 保留arity、ordinary assignability与exact projection参数校验；wrong arity/type仍必须报原有或更精确
   diagnostic。
5. 某个parameter无法解析时必须报owner/slot diagnostic并fail closed，但不能因此静默丢弃已精确解析的
   return facts。call return ordinary/exact cache仍应保留，避免制造无关field级联错误。
6. Agent形状的return同时记录：
   - ordinary owner-bound`PackageSymbol`；
   - exact`PackageSchema(packageId, stableKey, typeId)`。
7. 不恢复旧的“完全跳过parameter validation”行为，不改变call target、callable id或expected Local ABI。

优先抽取/复用现有：

- `canonicalize_package_signature_type_for_owner`
- `canonicalize_package_method_type_ref`
- `canonical_package_local_type_slot`

不能在`call_type`复制一套symbol字符串映射。

## 最小测试矩阵

在`compiler/tests/package_imports.rs`建立真实fresh A→B artifact-only fixture；consumer不能读取provider source
facts。至少覆盖：

- B公开owner-local、含interface成员的`Bindings`与schema-closed public `Result`；
- callable `(Bindings) -> Result`；
- A调用后放入unannotated local，读取scalar与nullable field并进入`JsonObject`；
- 直接从call result读field；
- B-owned nested record field；
- transparent alias field仍按RHS，alias无新schema identity；
- wrong arity、wrong nominal/scalar parameter、不存在field；
- owner/type slot歧义或缺失fail closed；
- private/nonexported local type不能经dependency alias恢复；
- lowered ref仍是原alias/callable id/expected Local ABI。

interface method return若暴露独立首次损失，只记录为后续节点；不得扩大本修复。

本节点唯一拥有：

```bash
cargo test -p skiff-compiler-source package_signature_local_slots_rehydrate_to_dependency_owner
cargo test -p skiff-compiler --test package_imports \
  dependency_callable_local_parameter_preserves_schema_result_field_types -- --exact
cargo test -p skiff-compiler --test package_imports
git diff --check
```

先用`--list`确认测试名与非零匹配；若最终采用等价名称，报告真实命令。不得运行完整compiler/workspace、
fresh生态、stable、live或chat smoke。

## 反向搜索、非目标与交付

提交前按F283列出的owner反向检查，尤其确认：

- 没有第二个“任一parameter含local slot便丢弃whole signature”的gate；
- ordinary与exact cache都更新；
- 没有新增artifact callable fallback或短名猜测；
- Agine/Agent源码零修改。

- 风险：中；验收组为后续W2-L combined compiler probe。
- worktree：`/Users/geek/workspace/skiff-p5-f285-field-read-fix`
- branch：`codex/p5-f285-field-read-fix`
- 不push，不操作stable。
- 从启动到第一次production修改不超过5分钟；不可执行时立即返回`TASK_NOT_EXECUTABLE`与最小前置。
- 完成后提交并返回commit、测试、自验收矩阵与设计缺口；不得自行承接W2-L。
