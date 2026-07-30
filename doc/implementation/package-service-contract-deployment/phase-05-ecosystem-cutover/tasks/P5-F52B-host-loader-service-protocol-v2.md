# P5-F52B：Host Loader Service Protocol v2

DAG节点F52B，依赖D52 COMPLETE。独立worktree，唯一production写入为
`runtime/host/src/loader/runtime_config.rs`及其聚焦测试。

service unit/contract identity admission只接受canonical
`skiff-service-protocol-v2:sha256:<64hex>`，使用artifact identity既有typed/prefix owner；真实v2正例，legacy
v1、坏长度、大写负例均fail closed。不得修改register mapper或`runtime.register.protocolVersion`（被设计决策
阻塞），不得改spawn producer/model/wire或新增兼容。运行loader聚焦测试、check/rustfmt/diff并提交单一commit。
禁止I02/R05/instance/stable/full gate/push/merge。
