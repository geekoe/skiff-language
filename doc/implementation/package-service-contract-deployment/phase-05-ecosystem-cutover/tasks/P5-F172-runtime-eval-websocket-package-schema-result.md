# P5-F172：Runtime Eval WebSocket Package Schema Cutover Result

状态：Completed

## 直接父任务

- `P5-F172-runtime-eval-websocket-package-schema.md`

## 交付

- `PinnedWebSocketContractPlan`现在只从service-call target固定的shared Package schema records编译Event、
  Result与Context value plan，不读取ServiceContract内嵌schema、artifact store或provider source。
- WebSocket canonical ABI验证显式消费同一admitted record closure；缺record以及Package owner、
  `stableSchemaKey`或`PackageSchemaTypeId`不匹配均在连接执行前fail closed。
- `ContractTypeRef::PackageSchema`仅与File IR规定的opaque `unknown` execution leaf匹配；其他linked type拒绝。
  Package完整身份由admitted record与contract ref共同固定，不从opaque execution leaf反推。
- receive Context codec与connect response codec使用已验证Package Context的content-addressed
  `PackageSchemaTypeId`；null Context、payload segment、消息编码、连接策略、错误及关闭语义保持不变。
- 测试fixture改用真实Package schema record和content-addressed type id，不再构造service-owned
  `ContractTypeId`或`boundarySchema`。

## 验证

```text
cargo check --locked -p skiff-runtime-eval --lib
passed

cargo test --locked -p skiff-runtime-eval websocket
16 passed; 0 failed

cargo test --locked -p skiff-runtime-eval
84 passed; 0 failed

git diff --check
passed
```

聚焦覆盖typed Context成功、缺record、owner/key/id逐项错配、非opaque linked Context拒绝、receive codec
identity错配、connect Context encode/decode及null Context。

范围文件反向搜索`ContractTypeId`、`ContractSchemaType`、`boundary_schema`、`PackagePublic`、
`ContractTypeRef::Contract`与`WebSocketIngressContext::Contract`均为零命中。

`cargo check --locked --workspace`已越过全部F172范围文件和完整runtime eval，当前首错属于后续
test-runner consumer：`ecosystem_smoke_fixture.rs`与`package_test_assembly.rs`尚未给
`project_service_deployment`传Package records，并仍构造已删除的definition `boundary_schema`字段。
