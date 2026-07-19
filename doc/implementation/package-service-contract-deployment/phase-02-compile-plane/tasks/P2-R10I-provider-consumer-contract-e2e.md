# P2-R10I：Provider / Consumer Contract E2E

## 目标

用真实package source证明同一ServiceContract可以分别编译provider wrapper与consumer，且typed contract事实
完整进入PackageArtifact，不读取或绑定provider deployment。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“四对象模型”“ServiceContract nominal
types”“调用模型”与“Package编译”章节。

## 依赖与写域

- 依赖P2-T03E、P2-T03G、P2-T04B、P2-R10H；先前6/7失败证据只用于证明旧候选断链，不能替代恢复后的运行。
- 独占`compiler/tests/service_conformance.rs`及必要的专用测试fixture；不修改production或common API。

## 完成态

1. provider fixture的公开wrapper使用如`payments.Request`的contract nominal type；断言精确
   `ContractRequirement`、Local ABI中的`PackageTypeRef::Contract`、与contract body匹配的Available boundary
   projection，且没有service runtime edge/provider/deployment字段。
2. consumer fixture同时使用contract nominal type和`payments/echo(input)`；不提供provider package，断言唯一
   requirement/slot/used operation/ServiceCallRef、合法File IR service-call external ref及Contract signature。
3. negatives至少覆盖unknown contract type、unknown operation、wrong argument/return use，以及package/contract
   alias冲突在trust boundary失败。
4. artifact反向断言不含provider build/package/deployment/route/executable target；不通过JSON字符串猜测主语义，
   typed assertion为owner，必要的wire反向搜索只作补充。

## 聚焦验收

- 只运行`service_conformance`及直接依赖的最小compiler test/check，随后`git diff --check`。
- 不运行全量foundation/compiler gate；T07统一运行。

## 禁止项

- 不为测试改production语义，不新增fake/empty contract或provider inference，不删除负例绕过失败。

## 执行合同

- DAG：波次9c恢复后的集成验收节点；完成后解除T07/A01。风险：高；动态证据由本任务唯一拥有，A01只读复核。
- worktree：`/Users/geek/workspace/skiff-p2-r10i-contract-e2e`；分支：`codex/p2-r10i-contract-e2e`；
  当前worktree保留首次失败测试；恢复时必须安全合入T03E/T03G/T04B checkpoint，不另建第二套fixture。
- 启动后5分钟内完成第一次实际测试代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；任一typed contract production owner、common
  fixture、ServiceContract/PackageArtifact schema或compiler pipeline变化即失效。
