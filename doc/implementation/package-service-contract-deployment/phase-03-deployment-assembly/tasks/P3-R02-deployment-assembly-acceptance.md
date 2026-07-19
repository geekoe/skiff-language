# P3-R02：Deployment / Assembly Checkpoint Acceptance

## 角色、输入与证据状态

- 独立只读验收 Agent；不得参与 T02/T03开发、修改文件或创建 commit。
- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§5、§6.2、§9、§10、§11、
  §12、§14。
- 主 Agent提供 exact clean integration commit；该 commit必须包含 R01已验收状态和 T02/T03，不包含 T06及
  Wave 3写入。
- 同时读取 T02/T03任务合同、自验收矩阵和聚焦证据；可运行最小风险探针，不重复完整阶段 gate。
- 证据只对 exact commit的 deployment/assembly surface、resolver/projection、dependency、fixture和测试状态有效；
  任一变化按边界使对应 verdict失效。

## DAG 与风险

- 同一 Agent/commit分别验收两个高风险边界，避免重复上下文，但 verdict必须分开。
- 只有 deployment与assembly verdict都 PASS才整体 R02 PASS并解锁 T06；单边 FAIL只退回对应 T02或 T03 owner。
- 当前成熟度：两个 implementation checkpoints；PASS不代表 runtime-link或阶段稳定候选。

## Deployment 必验

- source-free typed producer；public path只在 trust boundary解析，operation完整映射。
- boundary Available、descriptor/ContractTypeId/value plan/effect与全部 implementation requirement精确验证。
- exact package binding、contract-only service selector、config literal/secret ref/state/resource/policy与 identity正确。
- missing/duplicate/extra/Unavailable/mismatch/unknown/ambiguous全 fail closed，无 compiler/legacy adapter依赖。

## Assembly 必验

- roots、A↔B cycle、package closure、唯一 local provider、exact build/version/local ABI和global ingress正确。
- 同 build单 code slot；per-activation service/config/state/resource template独立，key包含 activation/caller build。
- service call保持 activation-relative ref/slot，不全局 patch provider executable；无 RemoteBoundary/fallback。
- empty assembly与 identity/order稳定；zero/multi provider、conflicting edge、tampered/dangling template均失败。

## 输出

第一行 `PASS` 或 `FAIL`；随后必须各写 `DEPLOYMENT: PASS|FAIL` 与 `ASSEMBLY: PASS|FAIL`。FAIL列 blocking
issue、设计/任务/production证据、影响、建议 T02/T03 owner及失效证据；另列 non-blocking、聚焦命令、动态风险和
exact commit。
