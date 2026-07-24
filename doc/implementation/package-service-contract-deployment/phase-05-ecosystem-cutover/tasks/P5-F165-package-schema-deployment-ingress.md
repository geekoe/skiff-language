# P5-F165：Package Schema Deployment 与 Ingress

状态：Ready

## 直接父任务

- `P5-F163-package-schema-projection-recovery-result.md`

## 范围

修改deployment projection、artifact-model WebSocket调用方及相关fixtures。不得修改compiler input/source、
runtime boundary/eval或consumer service。

## 必须实现

- 删除`canonicalize_service_owned_operation_contract`；PackageArtifact boundary operation与
  ServiceContract operation直接按Package-owned refs比较。
- deployment projection从canonical store或上游已验证输入取得ServiceContract所需的精确
  `PackageSchemaTypeRecord`闭包，不从provider源码、active deployment或version猜测。
- WebSocket ingress显式接收resolved records验证Context/Event/Result的persistable closure；缺record、
  foreign owner、contract未require、hash/closure错配均fail closed。
- HTTP ingress继续只绑定operation；不得在deployment/Router重建HTTP结构类型。
- 删除fixture中的`boundary_schema`与service-owned type构造，使用真实Package records。

## 验证

- deployment crate恢复编译并通过operation/ingress聚焦测试。
- operation owner/ref完全一致为正例，owner/key/id任一变化拒绝。
- WebSocket Context requirement存在且records完整为正例；缺失/foreign/non-persistable/SCC拒绝。
- Registry旧package-public/service-owned descriptor mismatch不再存在。
- `git diff --check`；独立提交并写result，记录runtime下游断面。

