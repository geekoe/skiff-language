# P5-F48A：I02 Fixture Effect Split

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第5、7、10、11条及§6–§8、§14。

DAG节点F48A，依赖D48，与F48B/F48C并行。独占I02 fixture/API与scripts direct tests：

- 保留source-local纯`marker()`供websocket调用，不返回spawn receipt token；
- 新增唯一suspending`submitSpawnReceipt()`执行canonical spawn，public `marker` API精确绑定该callable；
- websocket operation仍绑定`main.websocket`且只调用纯marker；
- 保持2个public operations、同contract/deployment、3个receipt entrypoints及既有identity规则；
- projection evidence断言public marker may_suspend/cooperative，websocket non-suspending/notCancellable；
- typed receipt不能从WS路径泄漏，不放宽compiler effect/ABI规则。

写入限于`test-runner/fixtures/package-service-i02-spawn-submit/**`、相关fixture projection test及
`scripts/tests/package-service-i02-combined.test.mjs`必要断言。禁止修改compiler/Runtime/Router/shared wire。

开发owner运行focused fixture projection/test-runner test、Node I02 direct与diff check；不得运行完整I02/R05/instance/stable。
独立worktree，5分钟内修改，提交自验收矩阵，不push/merge main。
