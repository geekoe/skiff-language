# P2-R06：Complete Canonical Package Requirement Closure

状态：rebuild；旧 commit `6274313` 只作只读证据，在 terminal T05 checkpoint 上重做 canonical 部分，
不得移植 runtime witness、legacy ABI 或 service resolver。

## 目标

`PackageArtifact.packageRequirements` 必须覆盖它所引用的全部外部 package ABI。当前显式
manifest dependencies 已进入该闭包，但编译器内建解析的 `skiff.run/std` 会生成合法
File IR `PackageRefIr::PackageId` 而没有 canonical requirement。本任务补齐 canonical owner，
不对 std 建立旁路或硬编码豁免。

## 完成态

1. 非 std package 的 File IR 实际引用 std 时，canonical `PackageArtifact` 包含
   `alias=std`、精确 package id/version 和 `expectedLocalAbi` 的正常 `PackageRequirement`。
2. std version/local ABI 只从同一轮 package graph 中已编译、已验证的 std
   `PackageArtifact` 取得；不硬编码 version/identity，不从 File IR 文本或旧 runtime DTO 推导。
3. std 未被引用时不增加 requirement；编译 std 自身时不生成 self requirement；正常
   用户配置仍禁止显式声明平台 std。
4. canonical materialization 校验每个 File IR `externalRefs.packageSymbols` 和
   `packageOperationSymbols`：`Dependency` 必须由 requirement alias 覆盖，`PackageId` 必须由
   requirement package id 覆盖。未知 alias/id 和未重写的 external self ref fail closed。
5. 覆盖校验只处理 dependency coordinate，不采集 symbol path/kind，不生成旧 used-symbol closure；
   后续终态 consumer 直接信任该 canonical invariant，不复制规则。
6. 现有单次 `PackageCompileInput -> PackageArtifact` 流程不变；不二次分析、lowering
   或 projection，不新增 artifact wire 字段。

## 写入范围

- compiler driver 中 canonical package requirement 构建、package graph/cutover 的必要窄调整及直接测试。
- canonical PackageArtifact materializer 的 File IR requirement-coverage 校验与聚焦测试。
- 不修改 source/lowering 语义、artifact wire/identity 算法、runtime linker、
  test-runner 或 checker allowlist。

## 验证

- 聚焦测试：used std 生成精确 requirement；unused/std self 不生成；std artifact 缺失或
  identity/version 不匹配 fail closed。
- canonical materializer 反例：未知 dependency alias、package id、external self ref 均拒绝；
  已覆盖 direct ref 与额外 transitive graph 不退化。
- 反向搜索证明没有 std ABI 硬编码、std 特例旁路或新的 symbol closure collector。
- 运行相关 compiler/emission 聚焦测试、必要 `cargo check` 和 `git diff --check`。
