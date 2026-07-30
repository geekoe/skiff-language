# P5-F50A：Host Spawn Continuation Probe

DAG节点F50A，依赖D50 COMPLETE。权威设计为
`doc/architecture/package-service-contract-deployment.md` §12 actor/spawn correlation条款。

从integration checkpoint创建独立worktree。唯一写入范围为`runtime/host`测试代码；不得修改production逻辑。
新增一个有界内存闭环测试，使用真实canonical assembly request/eval、共享OutboundRequestRegistry与
router-session dispatcher：

1. dispatch `request.start`；
2. 捕获outbound `spawn.submit.request`及rpcId/完整ActivationIdentity；
3. 向同一dispatcher注入严格typed `submitted` response与stable spawnId/itemId；
4. 在短timeout内断言同一request产生唯一`response.end`和fixture业务payload；
5. 断言outbound lease/pending与request supervisor归零，不需要worker registry。

至少覆盖错误rpcId不唤醒或cancel/receipt竞态中的一个关键负例。若无法用现有测试fixture构造完整canonical
request，应在5分钟内报告`TASK_NOT_EXECUTABLE`与最小缺失 seam；不得扩张到production修改。运行命名测试、
changed-file rustfmt与`git diff --check`，提交test-only commit并返回精确PASS/FAIL断点。禁止I02/R05/instance/
stable/full gate、push或merge。
