# P2-T03D：Terminal Service-call Lowering

## 目标

让lowering只消费source已经解析并typecheck成功的contract call target，删除lowering自建的第三份contract
operation index和沿调用链传递的重复参数。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“调用模型”与“Package编译”章节。

## 依赖与写域

- 依赖P2-T03A；可与P2-T03B并行。
- 独占`compiler/lowering/**`、driver中只为旧lowering index存在的handoff及lowering直接测试。
- 不修改source type/call analysis、projection或integration fixture。

## 完成态

1. 删除`contract_dependency_operation_index`及其公开类型、构造器、参数链和driver projection。
2. lowering从`ResolvedCallTarget::ContractOperation`直接取得精确`ContractRequirement`与
   `ContractOperationId`；protocol只读取requirement中的identity并写入`ServiceCallRef`，不保留第二字段。
   随后分配稳定slot并生成现有终态`ServiceRequirement`/`ServiceCallRef`。
3. 只声明未调用的contract dependency仍只产生`ContractRequirement`，不产生service requirement或call ref。
4. unknown/untyped/identity不一致的target fail closed；不得从callee path或display string重新lookup。
5. used operation去重与slot稳定性保持canonical，artifact中没有provider/build/deployment/executable target。

## 聚焦验收

- lowering direct tests覆盖一个调用、同contract多operation、重复调用去重、仅声明未调用和非法target。
- 反向搜索证明lowering operation index类型/模块/driver handoff归零。
- 运行lowering/driver最小检查及`git diff --check`，不运行Phase gate。

## 禁止项

- 不保留legacy/compat index，不把source错误延后为runtime错误，不选择provider。

## 执行合同

- DAG：波次8b、与T03B并行；完成后等待T03C/T04A并共同解除R10I。风险：高；typed-contract production
  独立验收组。
- worktree：`/Users/geek/workspace/skiff-p2-t03d-service-lowering`；分支：
  `codex/p2-t03d-service-lowering`；从含T03A的integration HEAD创建，禁止复用旧worktree。
- 启动后5分钟内完成第一次实际代码修改；否则回报`TASK_NOT_EXECUTABLE`，修改前不跑测试。
- 提交一个聚焦commit和自验收矩阵。证据只对该commit有效；resolved target shape、lowering service-call
  owner或driver handoff变化即失效。
