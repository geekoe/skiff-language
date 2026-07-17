# Phase 02：PackageArtifact 与唯一代码编译线

状态：outline-only；Phase 01 验收后再细化

## 输入

- Phase 01 的 canonical identity、type closure、typed effect leaf 与单一 PackageUnit projection。
- `doc/architecture/package-service-contract-deployment.md` 的 PackageArtifact、Local ABI 与 boundary
  eligibility 契约。

## 目标

- 用 `PackageCompileInput -> PackageSourceModel -> LoweredPackage -> CompiledPackage` 取代共同
  publication compile pipeline。
- 生成最终 `PackageArtifact`、`PackageLocalAbi`、callable semantic facts、config/resource/capability
  requirements 和显式 boundary projection。
- 实现 sound conservative callable may-effect/provenance 分析；mutable、alias、escape、same-heap 或
  unknown callable 保持合法 Local ABI，但 boundary 为结构化 `Unavailable`。
- 旧 service source path只能调用 package compiler 后接薄 legacy adapter，不能保留第二套代码分析。

## 验收边界

- package 可以完全独立编译、发布和直接链接。
- compiler production code不再有 package/service 两套 source/type/lowering 规则；任何 service-only
  overlay 都不拥有用户代码事实。
- public callable 必有 Local ABI 和明确 boundary projection；缺字段不能代表未知。
- 本阶段不引入 ServiceContract authoring、deployment 或 runtime service binding。

## 细化前复查

重新盘点 Phase 01 后的 `PublicationInput`、source/lowering branch、effect fixed-point、PackageUnit wire
shape、package test path和旧 service adapter。直接踩到的新重复必须升级成 Phase 02 前置任务。
