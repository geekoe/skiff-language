# P5-F402 Service-call manifest selection design

状态：Complete（docs-only design update）。

## 目的

把service-to-service callable选择从`api.yml` leaf上的`serviceCall: true`移到
`service.yml.serviceCalls`数组，并保持Package public API与Service role选择分层。

本节点只修改权威设计，不实现compiler/artifact迁移。实现节点必须以
`P5-F402-service-calls-manifest-selection-design-result.md`为直接父节点，不得继续执行F400A的
service-only或“全部public callable自动暴露”方案。

## 权威设计owner

- `doc/architecture/package-service-contract-deployment.md`
- `doc/reference/api-yml.md`
- `doc/reference/static-semantics.md`
- `doc/architecture/compiler-package-pipeline.md`
- 由上述语义派生的runtime/artifact、interface carrier、registry与overview文档

## 完成条件

- `api.yml` ordinary public symbols只使用scalar source selector；public instance只保留
  `const/interfaces`结构。
- `service.yml.serviceCalls`只列`api.yml` public root path。
- PackageArtifact不含service selection；只改变列表不改变PackageArtifact/Local ABI identity。
- ServiceContract、deployment binding、remote public-instance carrier与文档统一消费同一选择。
- 明确保留权威设计中的package/service source-root ownership；本节点不把service root改为
  service-only manifest。
