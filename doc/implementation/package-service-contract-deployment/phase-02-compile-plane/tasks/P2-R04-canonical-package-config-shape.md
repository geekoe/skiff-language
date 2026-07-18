# P2-R04：Canonical Package Config Shape

状态：amended；只保留 PackageArtifact runtime requirements/ConfigShape 的 canonical 规则。
旧 PackageUnit/service presentation 消费不纳入 Phase 02 最终 tree。

## 目标

`PackageRuntimeRequirements.config` 是 package runtime config 语义的 canonical owner：`path`、
`valueType`、`required`。本任务冻结 requirements 与 `ConfigShape` 的唯一 typed 表达和校验，供
`PackageArtifact` 及未来 `ServiceDeployment` 直接消费。

## 完成态

1. value type 非法、重复 path 等 canonical shape 错误结构化 fail closed。
2. `ConfigShape` 只能从 canonical requirements 构造；不从 source seed、service config 或旧 DTO 回填。
3. Available boundary callable 的 config requirements 必须被 package runtime requirements 完整包含。
4. resource/runtime capability 等不同 requirement lane 保持 typed 区分，不降级为 config 字符串。
5. production 只有一个 shape validator/constructor；future deployment 复用该 owner，不复制规则。

## 范围与验证

- 可修改 artifact-model 的 config/requirement typed helper、canonical package projection 及直接测试。
- 不新增 PackageArtifact 字段，不从 seed/source/projection 回填 canonical facts。
- 不修改或新增旧 runtime DTO、adapter、service presentation。
- 运行 artifact-model、canonical emission 聚焦测试、compiler check 与 diff check。
