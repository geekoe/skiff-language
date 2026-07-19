# Phase 02：Compile Plane

状态：complete；已验收并通过 merge commit `629e78d` 合入 `main`；详见 `phase-result.md`

## 输入

- 唯一权威设计文档中的 PackageArtifact、ServiceContract、effect/boundary eligibility 与三类 dependency edge。
- Phase 01 已合入的 canonical identity、type closure 和 typed effect leaf。PackageUnit 不是本阶段输出或兼容目标。
- Phase 02 integration 从 `9ca2547` 创建并完成 terminal rebuild；该基线与旧 integration tail 仅是历史
  执行事实，不再表示当前待办。

## 完成态

- 独立生成并校验 `ServiceContract`，contract 可先于 provider 发布。
- 唯一 package compiler 生成最终 `PackageArtifact`；所有用户源码不再按 package/service kind 分叉分析。
- provider 与 consumer 只凭同一 contract 独立编译；consumer 不读取 provider package、deployment 或 route。
- public callable 有 Local ABI 和显式 boundary projection；sound may-effect/provenance 对 unknown fail closed。
- 实际 service call lowering 只生成 `ServiceRequirement`、binding slot 和 `ServiceCallRef`。
- compiler 不生成 PublicationAbiUnit、PackageUnit、ServiceUnit、serviceAssembly 或任何 compatibility adapter 输出。

## 预期波次

1. 共享 schema/identity/API checkpoint：ContractTypeId、BoundaryOperationDescriptor、PackageArtifact、
   ServiceContract、requirements 与 call refs。
2. 三域扇出：contract artifact；package source/effect pipeline；dependency lowering 与 artifact projection。
3. canonical fixture/DB schema 收敛、终态结构审计、批次 gate 与独立验收。

若细化后需要超过三个实现波次，必须先重新检查接口冻结和写入 ownership。

## 阶段验收

- `contract -> provider package` 与 `contract -> consumer package` 两条编译路径都不需要另一方源码/artifact。
- ordinary package direct call 继续允许 alias/mutation；boundary-unavailable helper 仍是合法 package API。
- compiler production tree 不再存在共同 publication source/type/lowering owner。
- 本阶段不选择 provider、不生成 deployment、不执行 service call；旧 service CLI/watch/runtime
  允许暂时不可用，不用兼容代码恢复。
