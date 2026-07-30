# P5-F48C：Runtime Submit Receipt Validation

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§7、§12及§14。

DAG节点F48C，依赖D48，与F48A/F48B并行。独占`runtime/host` actor client/adapter typed response consumer及聚焦tests。

`ActorClient::submit_spawn`必须在source continuation前验证typed response：

- status精确为`submitted`；
- spawnId/itemId满足现有canonical稳定identity/非空约束；
- rpc correlation与structured ActivationIdentity保持；
- bad status、missing/empty/invalid IDs、typed error全部fail closed，不能被adapter当成功。

不得修改wire DTO/Router/eval/fixture或worker route。运行focused host submit-response tests、host check与diff check；禁止真实
I02/R05/instance/stable。独立worktree，5分钟内修改，提交自验收矩阵。
