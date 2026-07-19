# P2-R05：Exact Legacy Package ABI Witness

状态：cancelled；terminal-only 决策后没有对应终态概念，不执行。

`PackageArtifact.packageRequirements[].expectedLocalAbi` 已表达 package compile/link 所需的精确依赖
witness。`ServiceUnit.packageAbiExpectations`、runtime-v1 `PackageUnit.abiIdentity` 及其转换逻辑不是
新架构输入，不能进入 `PackageArtifact`、`ServiceContract` 或测试设施。

旧 integration 中的本任务提交不进入新 branch。无需 replacement task，也不得在 Phase 03 改名恢复。
