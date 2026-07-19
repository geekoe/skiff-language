# P2-T03I：Interface File IR Execution Projection

## 目标

让File IR interface operation signature消费T03H exact facts并复用T03G唯一execution projection，删除从interface
AST/type text生成contract `ServiceSymbol`的第二lowering owner。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”中File IR execution type
representation规则与“Compiler 与 Projection 流水线”章节。

## 依赖与写域

- 依赖T03G、T03H。
- 独占`compiler/lowering`中的interface declaration signature handoff、execution projection复用与direct File IR
  tests；只读消费T03H source API。
- 不修改source语义/fact shape、compiled/projection-input、PackageArtifact projection、artifact schema或integration
  fixtures。

## 完成态

1. interface declaration lowering不再从AST type text独立lower参数/返回，只消费T03H exact interface facts。
2. interface与executable复用同一个`PackageTypeRef -> TypeRefIr` execution projection：Local保真、
   Container/Nullable递归、Contract leaf固定为无参builtin/native `unknown`；不得复制规则。
3. method name/order、params、return、generic/Self替换后的execution shape和method flags保持；missing/duplicate
   source fact或unsupported shape fail closed。
4. contract来源的alias、stable key、`ContractTypeId`、display text和`ServiceSymbol`不进入File IR interface
   signature；external symbol空/非空不能改变结果。

## 聚焦验收

- direct File IR tests同时断言interface operation与impl executable的contract leaf均为opaque `unknown`，nested
  container/nullable一致，wire中无alias/`ContractTypeId`/`ServiceSymbol`。
- 反向搜索interface declaration的AST qualified type重算和contract `ServiceSymbol` fallback归零。
- 运行lowering/compiler最小测试/check、changed-file rustfmt和`git diff --check`，不运行Phase gate。

## 执行合同

- DAG：波次9e，与T04C按文件ownership并行；两者共同解除R10I evidence refresh与production复验。风险：高；
  interface execution handoff checkpoint。
- worktree：`/Users/geek/workspace/skiff-p2-t03i-interface-execution`；分支：
  `codex/p2-t03i-interface-execution`；从含T03H的integration HEAD创建。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；T03H facts、T03G execution projection、
  interface declaration schema或File IR identity变化即失效。
