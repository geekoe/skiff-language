# P2-R10I：Provider / Consumer Contract E2E

## 目标

用真实package source证明同一ServiceContract可以分别编译provider wrapper与consumer，且typed contract事实
完整进入PackageArtifact，不读取或绑定provider deployment。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“四对象模型”“ServiceContract nominal
types”“调用模型”与“Package编译”章节。

## 依赖与写域

- 依赖P2-T03E、P2-T03H2、P2-T03I、P2-T04C、P2-T04D、P2-R10H；先前6/7、7/7与波次9g的4/7
  证据都只具有历史价值，不能替代两项9h repair合流后的刷新运行。
- 只读消费`compiler/tests/service_conformance.rs`及专用测试fixture；不修改fixture、production或common API。

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

- DAG：波次9i evidence refresh节点；等待T03H2与T04D共同合流，与F09D production复验共同通过后解除T07。
  风险：高；动态证据由本任务
  唯一拥有，A01只读复核。
- worktree：`/Users/geek/workspace/skiff-p2-r10i-contract-e2e`；从同时含T03H2与T04D的同一integration HEAD以
  detached HEAD创建，只复用已合入的同一fixture，不另建第二套fixture。
- 本节点只刷新动态证据，不修改测试或production code，不创建提交。若现有fixture失败，直接回报精确FAIL，
  不在验收节点内修复或为通过测试改写断言。
- 返回精确commit、命令、结果和自验收矩阵。证据只对该commit有效；任一typed contract production owner、common
  fixture、ServiceContract/PackageArtifact schema或compiler pipeline变化即失效。
