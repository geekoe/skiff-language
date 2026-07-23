# P5-F52A：Router Service Protocol v2

DAG节点F52A，依赖D52 COMPLETE。独立worktree，唯一production写入为
`router/src/protocol/runtimeProtocol.ts`及对应Router测试。

把spawn submit request、spawn claim request、claim response item及retained legacy request.start中实际承载
ServiceProtocolIdentity的validator统一切换到既有`SERVICE_PROTOCOL_IDENTITY_PATTERN` v2，并更新字段语义对应的
fixture。每个面必须拒绝合法形状的legacy v1，并覆盖坏长度/大写；runtime frame schema v1继续接受。

不得改runtime.register/protocolVersion（被设计决策阻塞）、renew/complete/fail无SPI字段、producer或RuntimeAssembly；
禁止dual-prefix/fallback/转换。运行命名Router tests、type-check、diff check，提交单一commit。禁止I02/R05/
instance/stable/full gate/push/merge。
