# P5-F172：Runtime Eval WebSocket Package Schema Cutover

状态：Ready

## 直接父任务

- `P5-F169-runtime-callback-package-schema-cutover-result.md`

## 当前断点

eval WebSocket plan、ingress和response仍使用旧contract-owned类型分支，并且调用新的
`websocket_ingress_context`时未提供admission固定的Package records。

## 范围

只修改`runtime/eval`中的`websocket_contract_plan`、`websocket_ingress`、
`websocket_response`及其直接聚焦测试，并写result。不得修改普通/stream materialization、
artifact model、transport、loader、boundary、host或compiler。

## 必须实现

- WebSocket connect context、消息计划、response直接消费已解析Package records。
- `ContractTypeRef::PackageSchema`按完整owner/stable key/type id与linked runtime type匹配；
  删除旧`Contract`、`PackagePublic`分支。
- 适配当前`WebSocketIngressContext` Package schema变体；不得恢复service-owned变体。
- 缺record、身份错配、linked type不一致在连接建立前fail closed。
- 保持HTTP upgrade、连接生命周期、消息顺序、错误与关闭语义。

## 验证

- WebSocket相关eval聚焦测试；crate恢复编译后运行完整eval测试；
- typed context成功、跨Package同名隔离、owner/key/id错配及linked type错配覆盖；
- 范围文件旧符号无命中；
- `cargo check --workspace`首错越过范围文件；
- `git diff --check`；
- 独立提交并写`P5-F172-runtime-eval-websocket-package-schema-result.md`。
