# P2-T03E：Canonical Dependency Source Address

## 目标

恢复语言既有的dependency source address语义：type qualified path使用`.`，package/contract callable address
使用`<dependencyAlias>/<publicPath>`。删除把dependency call误写成`alias.member(...)`的实现与测试。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“ServiceContract nominal types”与
“Compiler 与 Projection 流水线”章节。用户可见既有语法证据见`doc/reference/any-interface.md`的“远程装箱源”
和`doc/reference/publication.md`的package dependency call示例；本任务不修改reference文档。

## 依赖与写域

- 依赖T03A–D及用户确认的`/`路径语义。
- 独占`syntax`中的dependency address AST/parser/helper、source dependency call resolution、contract-call tests，
  以及因AST终态重命名必须同步的窄compiler consumers。
- 不修改contract type semantics、executable signature facts、PackageArtifact projection或integration fixtures。

## 完成态

1. AST使用职责准确的`DependencySourceAddress { dependency_ref, public_path }`或等价终态名；删除
   `RemotePublicInstanceSource`过窄owner，不保留type alias/compat variant。
2. `alias/publicPath(...)`按validated alias解析为package callable或contract operation；public path可含规范化
   nested segments。`alias/publicInstance.method(...)`与`alias/publicInstance as I`继续工作。
3. `alias.Type`只用于qualified type；`alias.operation(...)`不作为dependency call兼容拼写，不能因call/type
   context猜测。
4. `/`无空白时才是dependency address postfix；带空白的`a / b`继续是除法。path canonicalization只有一个
   helper/owner，source与lowering不各自拼字符串。
5. package/contract alias共享namespace并在trust boundary冲突失败；slash本身不决定local/remote linkage。
6. 本任务既然直接修改contract-call checker，同时把超长`check_call`拆成lookup/shape/arguments/return职责，
   并把contract projected environment成组状态移出巨型`expression_type_model.rs`；T03F不得再修改这些owner。

## 聚焦验收

- parser/source tests覆盖package call、contract call、nested public path、remote public-instance direct call/boxing、
  dotted type与dot-call负例、slash/除法消歧。
- 反向搜索证明旧AST名和dot dependency-call fixtures归零。
- 运行syntax/source/lowering受影响的最小测试/check与`git diff --check`，不运行Phase gate。

## 执行合同

- DAG：波次9a共享语法检查点；可与T03F按文件ownership并行，完成后解除T03G/R10I。风险：高；进入
  typed-contract production复验组。
- worktree：`/Users/geek/workspace/skiff-p2-t03e-dependency-address`；分支：
  `codex/p2-t03e-dependency-address`；从调度时integration HEAD创建，禁止复用旧worktree。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试或宽泛盘点。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；syntax AST/path helper、dependency facts或call
  resolution变化即失效。
