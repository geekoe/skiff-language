# P5-F283 Dependency result field-read audit result

状态：COMPLETE；shared compiler blocker已定位，允许一个有界修复节点。

## 结论

首次损失不在Agent artifact、Agine dependency ingest、member lookup或object-literal target typing，而在
`compiler/source/src/expression_type_model.rs::OwnerChecker::call_type`对两个dependency call expression
做返回类型解析时：

```text
agent.thread.stopThread(...)       # 104:19
agent.thread.markDeleted(...)      # 125:19
```

当前代码已从`SourceDependencyAnalysisInput`找到完整且精确的callable signature，但因为第二个
`bindings`参数是`PackageTypeRef::Local(TypeRefIr::ServiceSymbol)`，第2812–2818行的整签名filter把
参数、返回类型一起丢弃。code-free artifact consumer又没有`TypeResolutionModel.package_callables`
fallback，因此两个`Expr::Call`同时得到：

- ordinary expression fact：`ExpressionTypeFact.ty = None`；
- exact expression projection：无`ContractProjectionState`条目。

随后的unannotated local binding、四个field expression和object-literal source field只是在传播这次
上游损失。最终四条“has no resolved expression type”由target-typed `JsonObject` materialization暴露，
不是首次失败节点。

最小引入commit是：

```text
a7af7209bb5a2f5baef202eddb1c3fa1b3d7b323
fix(test): close public test service migration
```

不是F268或F273。该commit在给artifact-backed public package call增加实参校验时，把原来的
“return不含local slot即可给call定型”改成了“return和所有parameter均不得含local slot”，却没有先把
dependency owner-local parameter rehydrate为consumer可解析的package-owned type。

## 审计边界与事实源

- production基线：`fe05440d345bf8bdeb6d750dd53efc0ee55d5b2d`。
- retained fresh store：
  `/tmp/skiff-f269-f278diag.DBqsFv/ecosystem-store`。
- Agent/Agine来源只读自F269 worktree；本文没有修改production、fixture、Internals、Agent、Agine、
  reference、architecture或其它任务文件。
- 本审计只执行了源码、artifact JSON和git history/diff读取；没有运行compiler/workspace测试、生态
  publish、stable、live或chat smoke。
- 架构结论沿用
  `doc/architecture/package-service-contract-deployment.md` §3、§6.1、§9：
  Package/Service API共用普通typed-expression机制，code-free dependency view保留owner Package，
  direct call的精确target先由validated dependency artifact解析，lowering不重建类型事实。
- alias与owner结论沿用`doc/reference/static-semantics.md` §1、§10、§16：
  transparent alias按RHS展开且不创建identity；dependency public API不能穿透private symbol；
  package schema nominal仍由声明Package拥有。

## 四条diagnostic与AST形状

真实source为
`/Users/geek/workspace/internals-p5-f269/agine/service/internal/agent_bridge_product_commands.skiff`。

| diagnostic | source | 表面expression | 第一个无resolved type的节点 |
| --- | --- | --- | --- |
| `stopped` | 107:14 | `stopped.stopped` | 104:19的`agent.thread.stopThread(...)` call |
| `runId` | 108:12 | `stopped.runId` | 同上 |
| `deleted` | 128:14 | `deleted.deleted` | 125:19的`agent.thread.markDeleted(...)` call |
| `stoppedRunId` | 129:19 | `deleted.stoppedRunId` | 同上 |

两个statement的AST均为：

```text
Stmt::Let {
  ty: None,
  value: Expr::Call {
    callee: Expr::Field {
      object: Expr::Field {
        object: Expr::Identifier("agent"),
        field: "thread",
      },
      field: "stopThread" | "markDeleted",
    },
    args: [
      Expr::Identifier("targetThreadId"),
      Expr::Identifier("runtimeBindings"),
    ],
  },
}
```

return value是`Expr::ObjectLiteral`；四个value均为：

```text
Expr::Field {
  object: Expr::Identifier("stopped" | "deleted"),
  field: "stopped" | "runId" | "deleted" | "stoppedRunId",
}
```

这些形状分别由`syntax/src/ast.rs::{Stmt::Let, Expr::Call, Expr::Field,
Expr::ObjectLiteral}`定义。执行传播如下：

1. `check_expr`仍会为call写入一个`ExpressionTypeFact`，但其`ty`是`None`。
2. `Stmt::Let`在无annotation时直接采用call的ordinary/exact结果；两者均为空，所以`stopped`和
   `deleted`没有进入ordinary env，也没有exact binding。
3. `Expr::Field`先解析receiver。identifier没有binding，故没有进入
   `TypeResolutionModel::record_field_type`，也没有进入exact `PackageSchema` child projection。
4. `Expr::ObjectLiteral`保存四个`ObjectLiteralSourceField { actual: None }`。
5. function return target是`JsonObject`。
   `materialize_target_typed_object_literal`只允许bare identifier在缺ordinary type时接受唯一contextual
   target；`expression_accepts_contextual_target`不接受field expression，于是四个位置分别报
   “object literal field ... has no resolved expression type”。

callee的嵌套`Expr::Field`在`check_callee_expr(..., diagnose_unknown_field = false)`中没有ordinary value
type是package path识别的既有工作方式，不是本回归首次损失；真正错误是随后已经找到的精确package
signature被filter抛弃。

## 按执行顺序的owner表

| 顺序 | 数据/节点 | production owner | 本案状态 |
| --- | --- | --- | --- |
| 1 | `api.yml` public path | Agent Package API projection | `thread.stopThread`、`thread.markDeleted`及两个result type均公开 |
| 2 | Local ABI public symbols/signatures | Agent `PackageArtifact.packageLocalAbi` | callable id、参数、return、type descriptor均精确存在 |
| 3 | Package schema identity/record | Agent schema index与逐类型record | owner、stable key、public path、type id和完整record descriptor一致 |
| 4 | dependency artifact选择 | `compiler/driver/source_compile/canonical_dependencies.rs::package_analysis` | 按Agine manifest alias `agent`、id/version选择并验证fresh artifact |
| 5 | callable/schema analysis facts | `package_callable_analysis_from_symbols`、`bind_callable_signature_identity`、`SourceDependencyAnalysisInput` | public callable signature和schema records均保留；无首次损失 |
| 6 | code-free ordinary type view | `TypeResolutionModel::build` artifact branch | public type/interface/type slot已索引；artifact branch没有索引callable |
| 7 | dependency call typing | `expression_type_model.rs::call_type` | **首次损失**：local parameter使整份exact signature被filter掉 |
| 8 | fallback callable lookup | `TypeResolutionModel::resolve_package_callable` | `package_facts: None`，所以`package_callables`为空，不能恢复 |
| 9 | local binding | `expression_type_model.rs::check_stmt`的`Stmt::Let` | call无ordinary/exact type，binding也为空 |
| 10 | field member lookup | `Expr::Field` + `TypeResolutionModel::record_field_type` + exact projection | receiver为空，两个owner均未被调用 |
| 11 | object-literal target | `ObjectMaterializationState`、`materialize_target_typed_object_literal` | 正确暴露四个missing actual；不是root cause |

`compiler/driver/source_compile/mod.rs::build`明确传入：

```text
package_facts: None
package_artifacts: Some(input.dependency_packages)
```

`TypeResolutionModel::build`仅在`package_facts`分支调用`index_package_callables`；artifact分支调用
`index_artifact_package_types`、`index_artifact_package_constants`等，但不建立callable fallback。精确callable
事实仍由独立的`SourceDependencyAnalysisInput`拥有，所以不应通过再建一个重复callable truth source修复。

## fresh Agent artifact精确证据

### Package级identity

| 字段 | 值 |
| --- | --- |
| package | `agine.ai/agent@0.1.0` |
| `PackageBuildId` | `skiff-package-build-v4:sha256:efbc104822cb278131fb4ae802889d425bb92ad1d95f426827b33ee294869845` |
| `PackageLocalAbiIdentity` | `skiff-package-local-abi-v3:sha256:5c43c2f3b47473af48d378a965d20a5e32ab2ea185290b9c654df8a6b3aefb90` |
| `PackageSchemaIndexIdentity` | `skiff-package-schema-index-v1:sha256:d68121afb54fa88f4f97f4eaea993063ef5757dd3bc7abdf90689ccaf35e7ace` |

pointer与artifact record分别位于：

```text
pointers/package-artifacts/agine~dai~sagent/0.1.0.json
records/package-artifacts/agine~dai~sagent/0.1.0/
  efbc104822cb278131fb4ae802889d425bb92ad1d95f426827b33ee294869845/package.json
```

### Result types

| public path / stable key | owner | `PackageSchemaTypeId` | 完整canonical record fields |
| --- | --- | --- | --- |
| `thread.StopThreadResult` | `agine.ai/agent` | `skiff-package-schema-type-v1:sha256:30910eb2fe51ffb2a72e12ef44dd9a28fef4fea285882a90da91626bbb216cf2` | `residualCleared: bool`、`runId: string?`、`stopped: bool`、`threadId: string` |
| `thread.MarkDeletedResult` | `agine.ai/agent` | `skiff-package-schema-type-v1:sha256:b7153d987ff0722eeb42cb3d847f2c79be9fea06d86f769229b5e94790773deb` | `deleted: bool`、`stoppedRunId: string?`、`threadId: string` |

schema index中的两项均为`publicNameable`且`publicPath`等于stable key。对应Local ABI public type symbol
分别是：

```text
localTypeId: type:thread.StopThreadResult
localTypeId: type:thread.MarkDeletedResult
kind: type
isAlias: false
isInterface: false
descriptor.kind: record
```

逐类型records分别位于：

```text
records/package-schema-types/agine~dai~sagent/
  30910eb2fe51ffb2a72e12ef44dd9a28fef4fea285882a90da91626bbb216cf2.json
  b7153d987ff0722eeb42cb3d847f2c79be9fea06d86f769229b5e94790773deb.json
```

### Callable signatures

`thread.stopThread`：

```text
callableId: pkg-callable:agine.ai/agent:thread.stopThread
parameters:
  targetThreadId: PackageTypeRef::Container("string")
  bindings: PackageTypeRef::Local(
    TypeRefIr::ServiceSymbol("tools", "AgentRuntimeBindings")
  )
return:
  PackageTypeRef::PackageSchema(
    owner = agine.ai/agent,
    stable key = thread.StopThreadResult,
    id = skiff-package-schema-type-v1:sha256:30910e...)
maySuspend: true
```

`thread.markDeleted`完全同构，return指向
`thread.MarkDeletedResult`/`skiff-package-schema-type-v1:sha256:b7153d...`。

触发回归的不是result type，而是第二个参数。`tools.AgentRuntimeBindings`本身在public Local ABI中是
`isAlias: false`的record type，但包含owner-local `any interface`等same-heap成员，因此合法地留在
`PackageTypeRef::Local`域，而不是获得伪造的`PackageSchemaTypeId`。修复不得强行把它投影成boundary
schema。

Agent source也与artifact一致：

- `packages/agent/api.yml`第22–23行公开两个callable，第70–71行公开两个result type；
- `packages/agent/thread.skiff`第56、178行声明result records；
- 第453、521行的free functions显式返回对应result type并接收
  `root.tools.AgentRuntimeBindings`。

## Agine ingest各跳保留/丢失

| 跳 | 输入 → 输出 | 保留 | 丢失 |
| --- | --- | --- | --- |
| manifest | `agine.ai/agent@0.1.0` → alias `agent` | id/version/alias，默认public access | 无 |
| artifact selection | pointer → exact PackageArtifact | build id、Local ABI id、public symbol surface | 无 |
| canonical dependency analysis | public Local ABI callable → `PackageDependencyCallableAnalysis` | callable id、semantic facts、完整signature；PackageSchema三元组不变 | 无 |
| schema binding | resolved schema index/records → dependency facts | owner、stable key、type id、descriptor、public path | 无 |
| ordinary type view | public Local ABI types → `package_types/package_type_slots/package_interfaces` | 两个result type与`AgentRuntimeBindings`的ordinary shape/owner view | callable没有在此重复索引；这是既有分工，不是artifact loss |
| call fast path | exact signature → call result | 应保存PackageSchema exact return并生成ordinary PackageSymbol | **因一个Local parameter丢弃整份signature** |
| fallback | package path → `resolve_package_callable` | 无 | artifact-only build没有`package_facts` callable index，返回`None` |
| expression/binding/member | call → let → field | 无可传播结果 | ordinary和exact cache均从call起为空 |

## 各类型在member lookup与expression cache中的owner

这里必须区分`PackageTypeRef::Local`这个Local ABI envelope与其内部的`TypeRefIr`种类；两者不是同一
“local type”概念。

| 类型 | ordinary expression/member owner | exact cache/identity owner | 关键规则 |
| --- | --- | --- | --- |
| `PackageTypeRef::PackageSchema` | `resolved_package_type_ref`把它表示为owner-bound `TypeRefIr::PackageSymbol`，再由`TypeResolutionModel::{package_type_resolution,type_shape_ir,record_field_type}`解析字段 | `ContractProjectionState`保存完整`packageId + stableSchemaKey + PackageSchemaTypeId`；`SourceDependencyAnalysisInput::exact_package_type`解析exact child | `TypeRefIr::PackageSchema`只用于显式serialization boundary；ordinary cross-package source reference按模型注释应是`PackageSymbol` |
| `TypeRefIr::PackageSymbol` | `TypeResolutionModel.package_types`，由artifact public Local ABI type index支持；lookup保留dependency/package owner与ABI expectation | 能找到schema record时可投影为`PackageTypeRef::PackageSchema`；否则保留`PackageTypeRef::Local(PackageSymbol)` | 不得按display string或短名猜owner |
| `TypeRefIr::PublicationType` | 由自身`module_path + type_index`选择`local_type_resolution`和`source_type_shape_ir` | source projection按实际声明展开/外部化 | slot owner是声明module，不是当前consumer module |
| `TypeRefIr::LocalType` | 由当前type context的`module_path + type_index`选择`local_type_resolution` | 同一source owner内可投影；跨artifact使用前必须按artifact owner rehydrate | dependency signature中的slot不能按consumer当前module解释 |
| `TypeRefIr::ServiceSymbol` | 当前source先查`source_types`；package signature场景可用artifact package type index按internal module/symbol恢复 | 若是contract alias可投影schema；ordinary package-local symbol应先绑定到唯一owner package | 本案`tools.AgentRuntimeBindings`属于这一类 |
| transparent alias | `expand_alias_type_ref`/`transparent_alias_ir`和`SourceTypeKind::Alias { canonical_target }`展开RHS后再做assignability/member lookup | Local ABI保留`isAlias`与RHS descriptor，但不创建`PackageSchema` nominal identity | F273已冻结；不能为修复本案给alias补identity |

字段访问正常路径同时维护两份事实：

1. ordinary receiver为`PackageSymbol`，`record_field_type`取回可继续做普通source checking的
   `ResolvedTypeRef`；
2. receiver exact projection为`PackageSchema`时，`Expr::Field`按精确schema record给child expression
   写`PackageTypeRef`。

本案在call节点同时失去两份事实，所以两条字段路径都没有机会执行。

## working / broken代码证据

### working predecessor

`a7af7209^`（即`48e5d8ce6b53c7692f7aab81fda21fd6b915c72e`）的package-call fast path只拒绝
return自身含unbound local slot：

```rust
.filter(|signature| !package_type_contains_local_slot(&signature.return_type))
```

两个Agent return均为精确`PackageSchema`，所以该状态会：

1. 生成ordinary `PackageSymbol(agine.ai/agent, thread.*Result)`；
2. 写入exact `PackageSchema` projection；
3. 给unannotated `stopped`/`deleted` binding定型；
4. 允许后续field lookup。

该旧路径没有验证public callable参数，这不是应恢复的终态，但它精确解释了“前序能过这些field、当前不能”。
F273结果commit
`d5645ea205af573788bfd2bdffc896098c223050`是`a7af7209`的祖先；其fresh chain已越过public alias位置并
进入更后的独立检查。F273修的是transparent alias projection，不是本filter。

### broken transition

`a7af7209`加入精确参数校验和dependency rebinding，同时把filter改为：

```rust
!package_type_contains_local_slot(&signature.return_type)
    && signature
        .parameters
        .iter()
        .all(|parameter| !package_type_contains_local_slot(&parameter.ty))
```

Agent signature判断结果为：

```text
return has local slot: false
targetThreadId has local slot: false
bindings has local slot: true  # ServiceSymbol
whole signature selected: false
```

同一commit新增的`bind_package_type_refs_to_dependency`目前只把适用的`PackageSymbol`重绑定到dependency
alias/expected Local ABI；`LocalType`、`PublicationType`、`ServiceSymbol`和`PackageSchema`均原样返回。
因此新参数filter并非多余保护：直接删除它再沿现有校验路径走，会把consumer的owner-bound
`agent.tools.AgentRuntimeBindings`与未rehydrate的`tools.AgentRuntimeBindings`比较，可能产生伪
nominal mismatch。正确修复必须保留参数校验并补齐owner-aware normalization。

最小回归引入范围就是`a7af7209`在
`compiler/source/src/expression_type_model.rs::call_type`的上述filter/validation hunk；不应回滚该commit
的其它test-service、lowering、loader或参数校验工作。

## 被上游失败遮挡的范围

| 风险面 | 本案是否直接覆盖 | 当前能否判定下游另有bug |
| --- | --- | --- |
| dependency-owned nominal record return | 两个call均返回Agent-owned public records | 被call loss遮挡；artifact/type index证据完整，修复后必须正测 |
| scalar field | `stopped`、`deleted` | 被遮挡；当前不能归因member lookup |
| nullable field | `runId: string?`、`stoppedRunId: string?` | 被遮挡；这里验证nullable projection/搬入JsonObject，不验证nullable dereference narrowing |
| object-literal target typing | 四个field value进入function return `JsonObject` | materializer正确暴露上游None；修复后必须证明不再报这四条 |
| local binding传播 | 两个call都先进入无annotation `const` | 直接覆盖 |
| nested dependency record field | result fields没有nested record | **未覆盖，保留为修复风险probe，不能宣称当前失败** |
| method/interface receiver return | source调用的是free public callable | **未覆盖，保留为修复风险probe，不能宣称当前失败** |
| transparent alias field/RHS | 由F273另行覆盖，但不在这四个expression中 | 本节点不重开alias设计；组合case仍应回归 |

因此不能用“只让这四条diagnostic消失”作为完成标准。上游call恢复后，若nested或method return probe另行
失败，应按最早损失重新界定，不得把额外语义悄悄塞进本修复。

## 最小production修复任务

### 必须达成的语义

在artifact-only/code-free dependency view中，public package callable signature的每个component应独立
解析：

1. 以`SourceDependencyAnalysisInput`中已验证的exact callable signature为唯一事实源。
2. 由dependency alias、package id、expected Local ABI和artifact type slots把
   `PackageTypeRef::Local`内部的`LocalType`、`PublicationType`、`ServiceSymbol`递归rehydrate为唯一的
   package-owned ordinary type；`PackageSchema`三元组保持原样。
3. 对可解析参数继续执行arity、ordinary assignability和exact projection校验；无法唯一解析owner/slot
   时产生明确diagnostic并fail closed，不能按短名或shape猜测。
4. 参数校验失败或某个local parameter无法解析时，不得静默丢弃一个已经精确可解析的return。compile仍因
   参数diagnostic失败，但call return ordinary/exact facts应保留，以避免下游级联成无关field错误。
5. 对本案return同时记录：
   - ordinary owner-bound `PackageSymbol`；
   - exact `PackageSchema(packageId, stableKey, typeId)`。
6. 不恢复“完全跳过参数校验”的旧行为，也不增加第二套artifact callable index。

`TypeResolutionModel::canonicalize_package_method_type_ref`、
`canonical_package_local_type_slot`与
`canonicalize_package_signature_type_for_owner`已经证明package owner/type slot能够转换成
`PackageSymbol`。实现应抽取/复用一个非interface专属、显式接收dependency owner的signature normalizer，
而不是在`call_type`中复制一组按symbol字符串匹配的规则。

### 唯一production owner与写入范围

最小允许范围：

- `compiler/source/src/expression_type_model.rs`
- `compiler/source/src/type_resolution_model/shape_assignability.rs`
- 仅确有必要时，对相邻`compiler/source/src/type_resolution_model.rs`增加窄API/错误类型导出

测试范围：

- `compiler/tests/package_imports.rs`中的fresh最小A → B fixture；
- 需要隔离owner/slot normalizer负例时，才在`compiler/source` co-located tests增加小单测。

不需要、也不应修改：

- `artifact-model/**`、artifact schema/identity/projection；
- `compiler/driver/source_compile/canonical_dependencies.rs`或重复dependency ingest；
- `syntax/**`、lowering、runtime、router、deployment；
- Agent/Agine source或fixture；
- F281 shared error DTO；
- reference/architecture或public language设计；
- 任何按`agine.ai/agent`、callable名、result名或四个field名写的特判。

### 反向搜索

实现节点在提交前至少反向检查：

```text
package_type_contains_local_slot
resolved_package_type_ref
bind_package_type_refs_to_dependency
bind_package_type_ir_to_dependency
canonicalize_package_signature_type_for_owner
canonicalize_package_method_type_ref
PackageTypeRef::Local
TypeRefIr::{LocalType,PublicationType,ServiceSymbol,PackageSymbol,PackageSchema}
resolve_package_callable
index_package_callables
package_facts: None
ContractProjectionState::record_expression_type
```

目标是确认不存在另一个“任一parameter含local slot便丢弃whole signature”的gate、第二套短名fallback、
或只更新ordinary/exact其中一份cache的路径。

## 正负测试矩阵

最小fresh A → B fixture应让provider B只通过生成后的artifact供consumer A使用，不让A读B source facts。

### 正例

1. B公开一个非boundary-schema、包含owner-local interface成员的`Bindings`，以及一个schema-closed
   `Result`；free callable为`(Bindings) -> Result`。A以`bindings: b.Bindings`参数调用它，先放入
   unannotated local，再读取scalar与nullable fields并返回`JsonObject`。这是Agent形状的最小复现。
2. 同一result含B-owned nested record，覆盖`result.child.id`。
3. 直接在call result上读field，覆盖没有local binding的expression cache传播。
4. B public interface method以owner-local parameter接收值并返回B-owned result，覆盖receiver/method
   return；若它暴露独立首次损失，应拆出后续节点。
5. result field使用transparent alias RHS，证明alias仍透明且没有新增`PackageSchema` identity。
6. 断言lowered dependency refs仍携带正确alias、callable id与expected Local ABI；不因source typing修复
   改写call target。

### 负例

1. wrong arity仍报callable arity mismatch。
2. 可解析parameter传入错误nominal/scalar type仍报argument mismatch；不能因保留return而吞掉错误。
3. 不存在的result field仍报unknown field。
4. owner/package id、stable key或`PackageSchemaTypeId`错配时fail closed。
5. local slot不存在、owner不唯一或module/symbol歧义时给出owner/slot diagnostic，不按同名类型猜测。
6. private/nonexported local type不能通过public dependency alias被恢复。
7. transparent alias仍按RHS工作且schema index中无alias identity。
8. fixture中搜索不到provider/package/callable/field特判和legacy/dual fallback。

### 最早probe与命令

建议新增test名：

```text
dependency_callable_local_parameter_preserves_schema_result_field_types
```

最早、最便宜probe：

```bash
cargo test -p skiff-compiler --test package_imports \
  dependency_callable_local_parameter_preserves_schema_result_field_types -- --exact
```

随后只运行有界owner tests：

```bash
cargo test -p skiff-compiler-source package_signature_local_slots_rehydrate_to_dependency_owner
cargo test -p skiff-compiler --test package_imports
git diff --check
```

不得把完整compiler/workspace gate或fresh生态publish用作实现循环。修复合入后，最终真实风险探针仍由
F269从fresh store重跑Agine与test-service总验收。

## 与F281 / F280 W2的文件关系和顺序

| 节点 | 与本修复的关系 | 调度结论 |
| --- | --- | --- |
| F281 / W1-S shared model | 只拥有`artifact-model`与`runtime/model`，删除`PackageCallableSignature.throw_types`等error DTO；本修复只消费params/return | 没有production文件重叠，也没有error-model语义依赖；开发可并行 |
| F280 W2-L language consumer | 明确拥有`compiler/source/**`（排除callable effects），覆盖本修复全部候选文件 | **不能并发修改**；必须串行 |
| F280 W2-A artifact consumer | 拥有`canonical_dependencies.rs`及artifact/contract consumer | 本修复不应写其文件；ingest已保留精确事实 |

明确集成顺序：

1. 最低风险方案是在当前pre-W1可编译integration上先完成并合入本有界修复及A → B test。
2. F281随后可合入strict shared-model checkpoint；W2-L必须从包含本修复的integration开始/rebase，并
   保留该regression test。
3. 如果F281先形成有意不可编译的strict checkpoint，则不要另开一个与W2-L并发编辑
   `compiler/source/**`的F283实现；把本修复明确交给W2-L，并在其第一个可编译状态先跑上述focused probe。

原因是F281删除shared DTO后，compiler在W2 consumer迁移完成前可以有意不可编译；这会阻断独立F283
focused test，但不表示F283语义依赖open error channel。F283也不得为了跨过该checkpoint自行恢复
`throw_types`或修改F281 DTO。

## 不允许源码workaround

Agine与Agent source无需也不得修改。禁止：

- 给`stopped`/`deleted`增加显式类型annotation来掩盖call fact缺失；
- JSON encode/decode、`JsonObject` round-trip、wrapper、字段复制或重复result type；
- 修改Agent callable参数/return或重新公开相同字段；
- 改为method/free function另一种写法绕开filter；
- package id、symbol、field特判。

annotation虽可能给local binding补ordinary type，却仍会掩盖call没有exact return projection，并让错误
parameter validation继续缺席；它不是语义修复。

## 设计判断

**无新增公共设计决策。**

现有架构已经要求：

- code-free service/package dependency API view复用同一typed-expression/member机制；
- Package direct call从validated PackageArtifact取得精确callable identity与Local ABI；
- schema nominal保留owner Package与`PackageSchemaTypeId`；
- transparent alias不创建identity。

这里存在的是production implementation/test ownership缺口，而不是用户可见语言缺口：

1. `SourceDependencyAnalysisInput`拥有exact callable signature，
   `TypeResolutionModel`拥有artifact type/slot view，但没有一个统一owner负责把
   `PackageTypeRef::Local` signature component rehydrate到consumer ordinary type域。
2. `a7af7209`把parameter representability错误地与return typing耦合。
3. 现有`package_imports`覆盖PackageSchema parameter/return和exact argument validation，却没有覆盖
   “Local ABI parameter + PackageSchema return + field read”的组合，所以回归逃逸。

最小修复应关闭这三个implementation invariant，不扩张artifact模型、公共语法或边界schema。
