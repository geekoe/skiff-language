# P2-F09D：Terminal Production Re-acceptance

## 目标

在含T03H1、T03I与T04C的同一最终候选上，独立只读确认exact contract signature、File IR execution
projection、public-instance handoff与PackageArtifact Local ABI已经收敛为单一production owner链。

权威设计：`doc/architecture/package-service-contract-deployment.md`的“Package”“Package-local ABI 与
Service ABI”及“Compiler 与 Projection 流水线”章节。

## 验收范围

- 输入任务：P2-T03F、P2-T03G、P2-T03H、P2-T03H1、P2-T03I、P2-T04B、P2-T04C。
- 只读检查`compiler/source`、`compiler/lowering`、`compiler/compiled`、`compiler/projection-input`与
  `compiler/projection/src/package_artifact`的相关production路径及直接测试。
- 确认source exact `ContractTypeId`事实经execution projection变成File IR opaque `unknown`，但PackageArtifact
  exact Local ABI仍只从source-owned callable facts获得；external package interface仍由typed package owner处理。
- 反向检查canonical路径不存在从File IR/alias-shaped `ServiceSymbol`重建exact ABI、public-instance
  `OperationAbiRef` owner、compatibility/fallback或第二套conformance计算。
- 对同一范围补充检查重复规则、缺失抽象、职责混杂和过长新增逻辑；非阻塞完美化建议不得扩大本阶段。

## 动态证据

- 运行完整`skiff-compiler-lowering`测试，确认历史34/35回归恢复。
- 运行`skiff-compiler-compiled --test public_instance_signature_handoff`及判断上述owner链所需的最小projection
  聚焦测试；不重复R10I的`service_conformance`，不运行T07宽gate。

## 执行合同

- DAG：波次9g只读production复验；与R10I evidence refresh共同通过后解除T07。风险：高。
- worktree：`/Users/geek/workspace/skiff-p2-production-reacceptance`；从integration最终候选以detached HEAD创建。
- 不修改文件、不提交、不修复；返回精确commit、PASS/FAIL、blocking findings、non-blocking follow-up、命令与
  残余风险。失败时给出最小可执行修复边界。
