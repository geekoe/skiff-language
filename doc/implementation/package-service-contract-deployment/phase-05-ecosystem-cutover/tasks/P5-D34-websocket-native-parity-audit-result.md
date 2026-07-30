# P5-D34：WebSocket Native Parity Audit Result

状态：complete。F23C1扩展driver测试的38个失败是同一个debug registry invariant级联。最早在F05
`c277e45`出现：四个WebSocket send首次进入`STD_NATIVE_CALLABLE_SEMANTICS`，但registry validator只接受
context-free、Date now与Time sleep，拒绝冻结语义要求的`Websocket → Websocket`。F23C/F23C1相关blobs不构成原因。

唯一production owner为`runtime/native/src/registry/table.rs` parity validator。direct-send下层路径完整，但debug/dev
runtime会在路由前全表assert，阻断F23D真实marker和R24。F23F已以exact修复合流：只允许四个WebSocket send的
`Websocket → Websocket`且handler数仍为0；File/HTTP/Actor/Telemetry/Resource保持fail closed，不新增source ACK。
