# P5-F445H I7 P8 A1 topLevelAlias instance method closure

状态：

```text
COMPLETE
RED_COMPLETE
A1_V_INTEGRATED
A1_COMPLETE
AGINE_170_RESUME_UNBLOCKED
```

## 1. Parent, baseline and ownership

- 直接父节点：
  `P5-F445H-I7-P8-D2-agine-top-level-receiver-authority-result.md`
- 架构事实源：
  `../../../../architecture/package-service-contract-deployment.md`
- ancestry floor：
  `2bcb40e61ee6b922eeca913651e2cc344a38b50e`
  （tree `df2bd49666a55d73f69b63b38c267bda8d2aed9d`）。
- dispatch时必须由主Agent提供D2已集成后的精确Skiff commit/tree；零worktree预检锚定该commit，不能
  读取移动中的integration工作区。
- repo：Skiff。
- integration owner：`/root/phase05_integration_steward`。
- DAG：
  `D2 -> A1 RED -> D4 -> A1-V -> A1 resume -> Agine 170 resume -> J`。
  A1与`S1 diagnostic -> S2 -> S3 -> I -> X`没有因果依赖，也不得修改或诊断P8 package stream链。

## 2. Exact semantic target

使用现有package artifact和call target模型闭合：

```text
kind:test direct topLevelAlias
  -> exact implementation type / AppliedNominal
  -> current impl method namespace
  -> PackageCallableId + exact PackageLocalAbi expectation
  -> CallTargetIr::PackageCallable
  -> receiver first execution arg
```

不新增语法、关键字、manifest/schema字段、artifact代际或动态method lookup。

冻结production写集：

```text
compiler/projection/src/package_artifact/callables/mod.rs
compiler/source/src/type_resolution_model.rs
compiler/source/src/expression_type_model.rs
compiler/source/src/resolved_call_targets/builder.rs
compiler/lowering/src/function_lowering.rs
```

冻结test写集：

```text
compiler/projection/src/package_artifact/tests/mod.rs
compiler/projection/src/package_artifact/tests/projection.rs
compiler/projection/src/package_artifact/tests/fixtures.rs
compiler/tests/package_imports.rs
test-runner/fixtures/alias-return-catch-once/main.skiff
test-runner/fixtures/alias-return-catch-once-tests/main.test.skiff
```

production文件不要求无条件全部修改；未修改项必须在result说明既有能力如何已满足。测试可以写在上述
production文件已有的`#[cfg(test)]`模块中。除此之外只允许机械更新同目录test module registration；
需要修改`callables/signatures.rs`、artifact model、projection input、resolved target enum、Runtime、
linker或其它owner时视为scope expansion并停止。零worktree先追踪现有signature、generic substitution、
receiver append和package callable link路径，再建立RED。

## 3. Required RED

第一笔test修改必须建立一个不依赖Agine的最小fixture。Compiler driver在
`compiler/tests/package_imports.rs`内构造同形临时project；真实linked execution复用已进入canonical
source suite的`alias-return-catch-once` subject/test service，只向上述两个fixture source文件增加同形
provider和一个case，不新建runner协议或registry：

Provider production source：

```skiff
type Box {
  value: string
}

function makeBox(value: string) -> Box {
  return Box { value: value }
}

impl Box {
  function read(self: Box) -> string {
    return self.value
  }
}
```

Provider的`api.yml`可以为空。Test service以同一dependency entry声明普通`alias: subject`和
`topLevelAlias: subjectImpl`，测试源码：

```skiff
import subjectImpl

test "package receiver method" {
  const box = subjectImpl/internal.makeBox("ok")
  assert box.read() == "ok"
}
```

RED必须在未改production的candidate上同时证明：

- provider artifact已有type与`makeBox`，但`implementationSymbols`/`callableLinks`缺
  `internal.Box.read` impl method；
- source无法把`box.read()`解析为package executable；
- failure先于Runtime执行，与HTTP stream、service boundary和test-runner编排无关。

只复制Agine错误文本、给Agine加wrapper、把method放进`api.yml`或直接调用Runtime不能替代此RED。

## 4. Required implementation closure

### 4.1 Projection

- 把当前production source set中的impl methods投影为稳定source method path对应的
  `implementationSymbols` callable；
- 使用现有`PackageCallableId`、exact callable signature、`OperationCallableKind::ImplMethod`和
  `callableLinks`，不得发明第二种method记录；
- signature沿用现有impl method projector对receiver与显式参数的定义，不能手工复制或篡改ABI；
- 重复method path、不可解析receiver owner或link target不唯一时fail closed。

### 4.2 Source typing and target resolution

- 仅当receiver type是direct top-level view产生的exact `TypeRefIr::PackageSymbol`，或以它为base且完成
  substitution的`AppliedNominal`时查询implementation method namespace；
- 同时校验dependency primary alias、direct top-level view、`abiExpectation`/expected build、完整
  receiver symbol path和唯一method identity；
- 解析结果使用既有`ResolvedCallTarget::DependencyPackageFunction`及精确
  `PackageCallableId`/signature，不按display string、短名或其它dependency同名method回退；
- 显式调用参数按排除receiver后的signature检查arity与类型；返回类型与generic substitution必须精确。

### 4.3 Lowering and execution

- 将上述resolved target降为既有`CallTargetIr::PackageCallable`，package ref canonicalize回该entry的
  primary alias；
- execution args顺序固定为receiver第一项，其后是源码显式参数；receiver不能重复计入source arity；
- generic `AppliedNominal`保留完整type arguments；
- 使用现有callable link完成真实linked execution，结果为`"ok"`。Runtime/linker生产代码默认NO-OP。

## 5. Mandatory negative matrix

同一测试族至少证明：

| 维度 | 必须fail closed或保持原dispatch |
| --- | --- |
| ordinary public alias | 未声明`topLevelAlias`的consumer通过public alias取得concrete type时，不能发现任意package-local impl method |
| service boundary | service call返回对象不能获得provider package-local method |
| interface | `any I`及package interface receiver只按interface slot，不回退到concrete method |
| permission | transitive dependency、缺`topLevelAlias`或非`kind:test`不能访问 |
| identity | ABI/build expectation不匹配、错误owner或同名method不能回退 |
| method | missing、duplicate/ambiguous或非callable path报结构化错误 |
| arity/type | 少参、多参、显式传receiver、参数类型错误均在source阶段拒绝 |
| generic | `AppliedNominal`错误arity/type args、未完成substitution或owner不一致均拒绝 |

还必须保留local concrete receiver、builtin receiver、actor receiver、public instance method和interface
method现有GREEN，不能改变它们的优先级。

## 6. Evidence

selector可按最终测试名机械调整，但result必须记录精确命令、非零发现数和结果：

```text
cargo test --locked -p skiff-compiler-projection package_artifact -- --nocapture
cargo test --locked -p skiff-compiler-source package_receiver -- --nocapture
cargo test --locked -p skiff-compiler-lowering package_receiver -- --nocapture
cargo test --locked -p skiff-compiler --test package_imports package_receiver -- --nocapture
cargo check --locked -p skiff-compiler-projection -p skiff-compiler-source \
  -p skiff-compiler-lowering -p skiff-compiler
node --input-type=module -e '
  import { runCanonicalSkiffSourceTests } from "./scripts/lib/skiff-source-test-suite.mjs";
  await runCanonicalSkiffSourceTests({
    registry: [{
      id: "top-level-alias-instance-method",
      root: "test-runner/fixtures/alias-return-catch-once-tests",
      subjectRoot: "test-runner/fixtures/alias-return-catch-once"
    }]
  });
'
cargo fmt --all -- --check
git diff --check
```

必须包含真实artifact projection、typed source fact、File IR target、receiver-first args和linked execution
五层断言；只做unit lookup或只让Agine compile不算完成。A1完成后可运行最便宜的Agine compile/discovery
探针，证明170个声明不再在同一编译点前被阻塞；完整170个默认测试仍归J。

## 7. Prohibitions and stop conditions

禁止修改：

- artifact/schema格式和identity代际；
- language grammar、keyword、`api.yml`、`service.yml`或package manifest schema；
- Runtime、linker、Router、test-runner production；
- Agine业务源码或测试以绕开failure；
- 普通alias、service boundary或interface权限；
- P8 stream lane的production、任务或根因。

若稳定RED无法建立、现有`PackageCallable`不能表达impl method、Runtime/linker确实需要production改动、
普通alias/service/interface语义必须改变、write set扩张到新的系统owner，或有多个会改变实现方向的方案，
立即返回`TASK_NOT_EXECUTABLE`或`TASK_SCOPE_EXPANDED`。提交已完成RED/诊断与精确缺口，不猜测性扩大。

## 8. Handoff

提交implementation与result，报告：

- branch、worktree、implementation/result commit/tree；
- 实际production/test/doc写集；
- RED与GREEN五层证据、negative matrix及generic结果；
- Runtime/linker/test-runner production是否保持NO-OP；
- Agine最便宜恢复探针与未运行的完整J gate；
- `A1_COMPLETE`、`AGINE_170_RESUME_UNBLOCKED`与scope状态。

交给`/root/phase05_integration_steward`串行集成、便宜探针和一级worktree/branch清理；不得自行写
integration、merge、push或启动J。

## 9. Development preflight

零worktree预检锚定：

```text
commit = 44e83695d5d9e6559b3ac5f482b9faffd1f96cb3
tree   = 6cc2284797d52a6d3549afb255eeaae6247a6915
```

预检确认单一路径可执行：

- `project_implementation_symbols`已经遍历每个File IR executable declaration，但当前只接受
  `ExecutableKind::Function`；在这里复用现有executable signature、`PackageCallableId`、
  `OperationCallableKind::ImplMethod`和callable link即可投影receiver method；
- top-level view已经把`implementationSymbols`索引进同一
  `SourceDependencyAnalysisInput`，并把`topLevelAlias`canonicalize回primary alias；无需新增dependency
  input或artifact字段；
- `TypeResolutionModel`已经保留exact package receiver的dependency ref、source symbol path、
  Local ABI和build identity；只需提供receiver method精确查询，不需要display-name fallback；
- `ResolvedCallTarget::DependencyPackageFunction`和`CallTargetIr::PackageCallable`已经携带本任务所需的
  callable identity；lowering只需把receiver field call识别为该typed target，并沿现有receiver路径把
  receiver放在第一项；
- Runtime/linker现有package callable link执行链无需修改。

实际production/test写集保持第2节冻结范围；没有运行中兄弟任务占有这些文件。第一笔实现动作先在现有
projection fixture、compiler driver临时project与canonical source fixture建立RED；若RED证明需要
`callables/signatures.rs`、artifact model、resolved target enum、Runtime、linker或test-runner production，
立即按第7节停止。

本节点风险为高（exact Local ABI identity与call lowering）。开发自验收拥有第6节聚焦命令；完整Agine 170与
阶段J仍不在本任务运行。证据仅对本分支最终implementation/result commit有效；production owner、fixture、
dependency artifact或identity输入变化即失效。

Resume预检锚定：

```text
commit = 6563ef36d7540636fe8e6b28ec4239d8845ec883
tree   = 383e9dff8e6b847a79dd899c205ad79f2c7bf293
```

该baseline已包含A1 RED checkpoint与A1-V。`implementation_link_callable_scopes`现在按
`OperationCallableKind`筛选link，并由`exact_callable_signature`在public/implementation Local ABI
surface中按canonical callable id唯一解析signature；原`implementationSymbols` validator blocker已解除。
恢复实现仍保持第2节冻结五个compiler production owner，artifact-identity不再属于本节点写集。

## 10. Scope expansion checkpoint and resume

A1 RED在第9节基线上稳定触发
`artifact-identity/src/package_artifact/validation.rs::implementation_link_callable_scope`的owner缺口。
该缺口由以下已拆分节点独占：

```text
P5-F445H-I7-P8-D4-implementation-callable-validation-authority-result.md
  -> P5-F445H-I7-P8-A1-V-implementation-callable-scope-validation.md
```

A1-V只拥有artifact-identity validator及其测试，不得修改本任务冻结的五个compiler production owner。
A1-V集成并设置`A1_RESUME_UNBLOCKED = YES`后，A1必须以该精确integration commit/tree重新做零worktree
预检，从已集成的A1 RED checkpoint恢复原第4至第6节闭环。恢复时不得把validator写集吸收到A1，也不得把
A1-V PASS写成`A1_COMPLETE`或`AGINE_170_RESUME_UNBLOCKED`。

只有恢复后的A1在原冻结写集上建立projection、typed source、File IR target、receiver-first args和linked
execution五层GREEN，才可设置：

```text
A1_COMPLETE = YES
AGINE_170_RESUME_UNBLOCKED = YES
```

在此之前，本任务保持`TASK_SCOPE_EXPANDED / RED_COMPLETE`。P8 stream lane的状态、任务和写集均不因
本次拆分变化。
