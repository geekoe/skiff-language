# Package、Service Contract 与 Deployment 总体实现计划

状态：active；Phase 01 已完成并合入 `main`，Phase 02 正从 terminal-only checkpoint 重建

唯一权威架构输入是 `doc/architecture/package-service-contract-deployment.md`。本文不定义新语义，只把
该设计转化为可逐阶段验收、尽量缩短关键路径的实现路线。每阶段只向 `main` 合并一次；上一阶段验收后
才细化下一阶段，但阶段内部通过短共享检查点和最多三个并行 worker 扩大 DAG 宽度。
阶段合并只保证本阶段负责的域已是终态；下游在后续阶段完成前可以暂时不可用。

## 1. 最终结果

最终生产数据流只有四个一等对象：

```text
ServiceContractDefinition ───────────────► ServiceContract
                                                │
PackageSource + required ServiceContracts ─────► PackageArtifact
                                                │
ServiceContract + PackageArtifact closure ─────► ServiceDeployment
                                                │
ServiceDeployments + PackageArtifact closure ──► RuntimeAssembly
                                                │
                                                ▼
                                         runtime replica
```

- 用户代码只由 package compiler 编译并归 `PackageArtifact` 所有。
- `ServiceContract` 无代码、可先于实现发布，是 consumer compile 的唯一 service 协议输入。
- `ServiceDeployment` 无源码，只把 contract operation 绑定到 typed package callable 和运行配置。
- `RuntimeAssembly` 解析完整 deployment/package 闭包；第一版所有 service edge 都是
  `InProcessBoundary`。
- `Publication` 不是领域对象或 compiler pipeline；`publish` 只保留为 registry 操作。
- package direct call 使用 Local ABI，可共享 heap、alias 和 mutation；service call 始终执行 boundary
  materialization 与 ActivationContext owner 切换。

## 2. 终态切换约束

不建立迁移 adapter ledger。一个生产域在所属阶段一次性切到终态，旧 owner 直接退出；
下游未完成时允许临时断链，不用兼容代码接回。

| 阶段 | 当阶段必须落地的终态 | 允许的临时状态 |
| --- | --- | --- |
| Phase 02 | compiler 只产出 `PackageArtifact` / `ServiceContract` | service CLI/watch/runtime 可暂时无法消费；不输出 `PackageUnit` / `ServiceUnit` / `serviceAssembly` |
| Phase 03 | 只用 `ServiceDeployment` / `RuntimeAssembly` 表达部署与闭包 | 尚未执行 service boundary；不保留旧 assembly/loader 兼容输入 |
| Phase 04 | `ActivationContext` / `InProcessBoundary` 是唯一 service 执行路径 | 工具与业务服务尚未切换；不保留 remote relay fallback |
| Phase 05 | CLI/watch/registry/router/test-runner/services 全部消费四对象 | 无旧 reader/writer、无 artifact 转换、无双轨 |

## 3. Phase 01 经验与后续约束

Phase 01 已建立 canonical identity、type closure、typed effect leaf、PackageUnit projection 和跨层引用验证。
其早期独立任务并发有效，但后期形成长串行链；单个跨层验证提交触及 89 个文件，证明“一个 Agent 横跨
compiler/runtime/router/scripts”不是可接受任务边界。

后续阶段必须遵守：

- 共享 owner/API 是阶段内部的短 checkpoint，不单独占一个长期阶段。
- checkpoint 后按写入域拆 consumer，最多同时运行三个开发 Agent。
- 每阶段默认不超过三个实现波次；若超过，先检查是否遗漏可提前冻结的接口或错误制造了串行依赖。
- 阶段边界选择可独立验收的控制面或执行面，不再机械地为每个 artifact 对象单设阶段。
- 不为维持阶段间可运行性建立 compatibility/legacy bridge；旧路径在其 owner 被终态
  取代的阶段直接删除。

Phase 01 的 `phase-plan.md` 与 `phase-result.md` 是当时执行记录，其中旧 Phase 02–07 编号只表示历史
ledger；后续以本文当前划分为准。

## 4. 当前阶段划分

```text
Phase 01  Canonical semantic / identity foundation（已完成）
    │
Phase 02  Compile plane：PackageArtifact + ServiceContract
    │
Phase 03  Deployment and assembly plane：ServiceDeployment + RuntimeAssembly
    │
Phase 04  In-process execution plane：ActivationContext + InProcessBoundary
    │
Phase 05  Ecosystem cutover：tooling、services 与 legacy deletion
```

### Phase 01：Canonical semantic / identity foundation

收敛 canonical JSON/framing、nominal/callable identity、package identity preimage、nominal type closure、
typed effect leaf、PackageUnit builder 和跨层 artifact reference validation。该阶段已验收并合入 `main`。

### Phase 02：Compile plane

同时建立最终 `PackageArtifact` 与独立 `ServiceContract`。先冻结二者共享的 boundary descriptor、
ContractTypeId、requirements、identity 和 wire API，再并行实现 contract artifact、package/effect pipeline
与 service dependency lowering。阶段完成后 provider 和 consumer 可只凭 contract 独立编译，consumer
artifact 只保存 `ServiceCallRef`，compiler 不再有 publication/package/service 共同 source pipeline，
也不产出任何旧 runtime DTO。现有 service CLI/watch/runtime 在 Phase 03–05 完成前可暂时不可用。

执行基线固定为 commit `9ca2547`：它包含 T01–T04 的 canonical contract、package artifact、effect 与
service-call lowering，但尚未引入后续 runtime 兼容链。旧 Phase 02 integration 只作只读取证；不从其
后半段整体 cherry-pick 或继续边删边修。后续终态实现按任务逐项移植或重做。

### Phase 03：Deployment and assembly plane

同时建立无源码 `ServiceDeployment` 与完整 `RuntimeAssembly`。先冻结 deployment/assembly schema、identity
和 binding template API，再并行实现 deployment projection、assembly closure/provider resolution 与 runtime
loader/linker adoption。阶段完成后 typed artifacts 足以构建、校验、链接和 admission 一个 assembly；
不读取 `ServiceUnit`、`serviceAssembly` 或其 adapter shape。

### Phase 04：In-process execution plane

建立 ActivationContext、service binding vector 和 transport-neutral materialization kernel；随后并行完成
ordinary/error、async/stream/cancel、callback/native capability 三类 lane。Ingress 与内部 service call
切到同一 dispatcher，package direct call 继续保留 same-heap mutation；production remote fallback 不可达。

### Phase 05：Ecosystem cutover

并行迁移仓库 tooling、`skiff-packages` 和 `internals` consumer：registry/release、CLI/watch、router reload、
test-runner、fixtures 与实际 services。每个 consumer 直接切到四对象，不通过旧 artifact adapter；
完成完整非 live verify、必要 live/smoke 与多 replica 验收。

## 5. Worktree 与提交协议

- 每阶段从最新 `main` 创建 `codex/package-service-phase-NN` 与 integration worktree。
- 当前阶段文档先在 integration branch 提交；task worktree 从该提交或后续明确 checkpoint 创建。
- 并行任务必须声明非重叠写入范围。共同 owner 先合成 checkpoint；consumer 不自行 cherry-pick 多个
  未集成分支拼装依赖。
- task Agent 完成并提交后，主 Agent 按 DAG 合入 integration branch并清理 task worktree/branch。
- 阶段 gate 与独立验收通过后，以一个 merge commit 合入 `main`。merge 后核对 tree 与证据，不机械重跑
  相同 commit 已通过的昂贵 gate。
- 跨仓库改动分别提交；未经用户明确授权不 push。

## 6. 验证层级

```text
任务级：format / 静态检查 / 直接 crate 或 test filter
批次级：共享 checkpoint consumer 组合 + 结构反向搜索
阶段级：受影响 subject selector + 架构 gate + 一次独立阶段验收
最终级：Phase 05 运行完整非 live verify 和必要 live/smoke
```

完整套件、冷构建、E2E 和 live gate 对同一稳定代码状态只指定一个 owner。删除或重写测试时必须说明旧
测试锁定的语义，并提供 replacement test 或证明该语义已整体删除。

## 7. 阶段调整规则

- 当前阶段发现会被本功能放大的重复、超长 owner 或隐式契约：拆成短前置 checkpoint，随后恢复并发。
- 只影响后续阶段：更新下一阶段 overview，当前阶段不提前实现。
- 会改变四对象边界、两类调用语义或本地 assembly 方向：暂停受影响 DAG 分支并请求用户决策。
- Agent 停滞、范围膨胀或跨越多个顶层写入域：保留有效提交，把剩余工作按 owner 重派。
- 评审只阻塞架构矛盾、不可执行 DAG、缺失 owner/删除条件或无法验收的问题；完美化和额外未来测试不
  阻塞。
