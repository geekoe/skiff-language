# P5-F23A：Router WebSocket Trust / Dispatch Seam

依赖D33 complete。与F23B/F23C并行；独占`router/src/protocol/runtimeProtocol.ts`、
`router/src/router/{assemblyRuntimeRegistry,runtimeDispatcher,runtimeEndpoint}.ts`及直接tests，不改gateway lifecycle、
Rust runtime、shared ABI或F03B generation consumer。独立worktree/branch，一个clean commit，不merge/push。

完成态：production registry接受且只接受canonical assembly WS unary；gateway只经dispatcher获取已校验的runtime
connection receipt，不直接依赖registry。pending request保存connect/receive phase，response decoder严格拒绝variant多余/
缺失字段、HTTP/WS metadata混用、非法payload flag；保留零字节typed Context合法组合。RuntimeEndpoint把发送方runtime
socket交给`connection.send` consumer，跨service/entry/sender为结构化protocol violation并隔离发送方，合法closed race记录
delivery-miss，禁止静默。Router selection前按冻结公式重算entry/gateway identity；不增加source ACK或改变std callable。

只跑Router type-check、protocol/registry/dispatcher/endpoint直接tests、response mutation与identity/sender负例、diff-check。
禁止full/I16/Host/stable。回报commit/tree、矩阵与clean状态。
