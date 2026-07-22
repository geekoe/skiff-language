# P5-F23B：Shared WebSocket Lifecycle Core

依赖D33 complete。与F23A/F23C并行；独占新的Router lifecycle模块、`webSocketGateway.ts`及直接tests，禁止修改
`assemblyWebSocketGateway.ts`、runtime protocol/dispatcher/endpoint或Rust。独立worktree/branch，一个clean commit。

从generic production owner抽出协议无关的唯一`WebSocketConnectionLifecycle`：connection/business indexes、原子policy
admission、每连接有界串行receive scheduler与cancel、连接上限、slow-client budget、runtime disconnect、close/deindex、
bounded shutdown与UTF-8 close reason。generic gateway先消费该core且行为不回归；默认policy close为1008，reject-new不先
upgrade，close-oldest先deindex。不得把generic wire composer抽入core，canonical零字节Context不能按payload长度丢presence。

使用同一参数化suite覆盖queue overflow、ordering、client close cancel恰一次、runtime disconnect、policy、backpressure、
limit与shutdown；跑相关Router tests/type-check/diff-check。禁止修改Assembly adapter、跑full/I16/Host/stable。
