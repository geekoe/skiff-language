# P5-F51A：Router Default Spawn Probe

DAG节点F51A，依赖D51 COMPLETE。独立worktree，唯一写入范围为Router测试代码，不改production。

使用真实默认`RuntimeRegistry`/`InMemorySpawnQueueStore`与loopback registered runtime，发送canonical
`spawn.submit.request`，在1秒内断言同rpcId、`status=submitted`、stable spawnId/itemId的typed response；
证明Mongo不可用/未启动不影响该路径。至少补一个store rejection转同rpcId typed error或socket发送边界关键负例。
运行命名Node测试、type/static check、`git diff --check`并提交test-only commit。

禁止修改store/endpoint production、I02/R05/instance/stable/full gate、push/merge。
