# P5-F165：Package Schema Deployment 与 Ingress Result

状态：Completed

## 直接父任务

- `P5-F165-package-schema-deployment-ingress.md`

## 交付

- deployment operation mapping删除service-owned canonicalization及对
  `skiff-compiler-contract`的依赖，PackageArtifact boundary operation与ServiceContract operation现在直接按
  Package-owned `packageId + stableSchemaKey + PackageSchemaTypeId`比较。
- `project_service_deployment`显式接收上游或canonical store已经解析的
  `PackageSchemaTypeRecord`集合：
  - 先验证record内容身份与无环闭包；
  - record集合必须与ServiceContract全部`PackageTypeRequirement`精确相等，缺失和额外record均拒绝；
  - requirement owner与record owner必须一致。
- WebSocket ingress把同一resolved record集合传给artifact-model的严格Context验证。Context必须被contract
  require，且其完整传递闭包存在、owner/key/id一致、可持久化并且无SCC。
- HTTP ingress路径没有schema特判或HTTP结构重建，仍只绑定contract operation。
- deployment projection、assembly与storage fixtures已删除`boundarySchema`、`ContractTypeId`、
  `PackagePublic`及service-owned构造，全部使用Package schema index/type refs和真实content-addressed id。
- 同步吸收F164之后的public-only schema index不变量，修复package id变更fixture的index owner，并将旧的
  非public key mismatch store测试改为入口直接拒绝。

## 验证

通过：

```text
cargo test -p skiff-deployment
48 passed; 0 failed

git diff --check
passed
```

聚焦测试覆盖：

- operation owner、stable key或type id任一改变均得到`OperationContractMismatch`；
- exact Package record closure正例，以及缺失、额外、foreign owner和descriptor/hash错配拒绝；
- WebSocket null与persistable Package Context正例；
- WebSocket CallbackInterface、Package schema SCC及contract未require child拒绝。

## 下游断面

`project_service_deployment`新增resolved Package records参数后，以下deployment调用方必须在各自owner任务中
从canonical store或当前编译projection结果传入精确闭包：

- `compiler/driver/generated_deployment.rs`与`compiler/tests/websocket_ingress.rs`；
- `test-runner`的package assembly与ecosystem smoke fixtures；
- `runtime/host`的assembly admission、execution与router session fixtures。

本任务没有为这些调用方增加空record fallback，也没有读取provider源码、active deployment或version来猜类型。
