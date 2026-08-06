# Runtime Lazy-Load Deployment — 实现计划

状态：实现计划。目标模型与平台侧契约见
`doc/architecture/runtime-lazy-load-deployment.md`。本目录存放本特性的任务描述
（leaf 文件），按里程碑拆分，后续补充。

## 1. 背景与基线

当前部署链路依赖 activation 协调层：watch 发布 → Mongo `skiff-router.activation_state`
（committed generation + pending）→ coordinator prepare/commit CAS → epoch store →
runtime 全量 assembly admission。该层为多 runtime 原子切换设计，在当前形态下产生过
真实故障：epoch 与仓库状态脱节导致激活 CAS 无限失败、504 语义歧义、watch 无限重建。

目标模型：不可变产物 + 小指针表 `(profile, serviceId, version) → buildId` + 注册目录 +
runtime 懒加载。激活协调层整体移除。

基线：本计划以 `main` 当前提交为基线；既有修复（capabilities 刷新、dev-sync 重试复用、
`assembly sync-state`、handshake corpus 计数）保持。

## 2. 实施阶段

### M1：release 指针表

- 在 typed pointer store 新增 release 指针键 `(profile, serviceId, version) → buildId`，
  复用原子写机制（rename + lock）。
- publish（`package publish` / authoring）在写 deployment 记录时同事务写 release 指针。
- 提供 CLI：`skiff release set/unset`（或并入 `assembly` 子命令族），含幂等与冲突校验。
- 验收：发布后指针可查；同 version 重发布原子覆盖；旧 buildId 记录保留。

### M2：runtime 懒加载

- 按 buildId 构建可执行 image 的路径复用现有 loader（deployment 记录 + package 闭包 +
  file-ir + config）。
- per-buildId 临界区：同 buildId 并发请求等待；加载失败/超时快速失败。
- 注册从"一个 active assembly"扩展为"已加载 buildId 集合"；能力通告（`runtime.capabilities`）
  携带 artifact root 与 lazy-load 标记。
- 已加载集合只增不删（逐出策略留待本地策略，不在本计划）。
- 验收：请求新 buildId 首次触发加载并成功执行；缺失 buildId 快速失败；并发请求不重复加载。

### M3：router 派发切换

- 请求解析改为 `(serviceId, version)` → release 指针 → buildId。
- 候选集 = 已注册该 buildId 的 runtime ∪ 具备懒加载能力且共享 store 的 runtime；
  fail closed 保持。
- 可选：fire-and-forget 预载提示（无 ack、无 pending）。
- 验收：新旧版本并行可路由；无候选时快速失败；同版本覆盖后新请求走新 buildId。

### M4：移除 activation 协调层

- 下线 `activation_state` 仓库、coordinator、epoch store、generation lease、
  config snapshot 独立提交。
- watch 改走"指针 + 幂等 deploy"；`assembly sync-state` 退役或改为指针表检查。
- 存量迁移：旧 committed 世代视为全部版本键的当前值一次性写入指针表。
- 验收：无 activation 残留引用；watch 全流程（变更 → 发布 → 指针更新 → 请求生效）无协调状态。

### M5：流水线与验证

- `skiff deploy`（publish + 指针更新 + 可选预载提示）/ verify / rollback（指针指回旧 buildId）。
- agine 侧验证：chat-smoke / two-hosts 全绿；同版本覆盖用例；回滚用例。
- 文档：架构文档与实现文档同步收敛。

## 3. 非目标

- 多 runtime 逐出/内存策略（本地策略，后续）。
- 指针表的分片/高可用（当前单实例足够）。
- 线上预载提示的默认开关策略。

## 4. 任务描述

按里程碑拆分的具体任务描述（leaf 文件）将追加到本目录，格式对齐
`doc/implementation/` 既有 leaf 约定（状态、改动范围、验收、无重叠约束）。
