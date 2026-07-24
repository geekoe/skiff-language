# P5-F190：Database State Requirement 投影

状态：Ready

## 直接父任务

- `P5-F164-package-schema-consumer-import-result.md`

## 问题与目标

真实 Registry package 声明并使用普通 database state `registry-store`，dev deployment 也提供绑定；
但 PackageArtifact 当前无条件生成空 `runtimeRequirements.resources`，使 deployment 报
`unexpected state binding registry-store`。恢复从 canonical source/package facts 到 PackageArtifact
runtime requirement、deployment binding 和 Runtime StateBinding 的唯一链路。

不得删除真实 deployment binding 绕过，也不得在 Runtime 按名字猜 DB。

## 验证

- 真实 Registry package build/deployment；
- 声明、有使用、无使用、缺绑定、多余绑定、错误 kind 矩阵；
- compiler/deployment/runtime 聚焦测试、workspace check、diff check；
- 独立提交和 result。

