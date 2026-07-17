# Phase 03：ServiceContract 与依赖编译

状态：outline-only；Phase 02 验收后再细化

## 输入

- 可独立编译的 PackageArtifact 与 boundary callable projection。
- 独立 PackageLocalAbi/PackageBuild identity。

## 目标

- 冻结 code-free contract authoring 输入并生成独立 `ServiceContract` artifact。
- 建立 `ContractTypeId`、closed boundary schema、operation descriptor 与
  `ServiceProtocolIdentity` 的唯一 owner。
- package 编译只读取 `ContractRequirement`；实际调用生成 `ServiceRequirement`、binding slot 和
  `ServiceCallRef`，不读取 provider package或deployment。
- 删除 `PublicationAbiUnit` 共同 aggregate；package local surface 与 service protocol 不共享父 DTO。

## 验收边界

- contract 可先于 provider 发布；provider 与 consumer 可只凭 contract 独立编译。
- 普通 A→B→A service call cycle 不形成 contract compile cycle；跨 contract schema closure cycle
  fail closed。
- structurally equal package type不能冒充 ContractTypeId；转换必须是 package 显式 wrapper。
- 本阶段不选择 provider、不生成 deployment，也不执行 service call。

## 细化前复查

基于 Phase 02 事实决定 contract authoring 文件与 CLI，但 artifact owner和 identity输入不得改变。复查
现有 contract schema、publication ABI、service dependency loader及registry存储，避免 dual source of truth。
