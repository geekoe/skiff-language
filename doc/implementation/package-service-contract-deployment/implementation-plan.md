# Package、Service Contract 与 Deployment 总体实现计划

状态：active；Phase 01–04 已完成，Phase 05 实现中

唯一权威架构输入是 `doc/architecture/package-service-contract-deployment.md`。本文不定义新语义，只把
该设计转化为可逐阶段验收、尽量缩短关键路径的实现路线。每阶段只向 `main` 合并一次；上一阶段验收后
才细化下一阶段，但阶段内部通过短共享检查点和最多三个并行 worker 扩大 DAG 宽度。
阶段合并只保证本阶段负责的域已是终态；下游在后续阶段完成前可以暂时不可用。

## 1. 最终结果

最终代码发布数据流只有四个一等artifact；activation另有独立配置snapshot输入：

```text
ServiceContractDefinition ───────────────► ServiceContract
                                                │
PackageSource + required ServiceContracts ─────► PackageArtifact
                                                │
ServiceContract + PackageArtifact closure ─────► ServiceDeployment
                                                │
ServiceDeployments + PackageArtifact closure ──► RuntimeAssembly
                                                │
selected three-layer config + exact closure ───► RuntimeConfigSnapshot
                                                │
                                                ▼
                                         runtime replica
```

- 用户代码只由 package compiler 编译并归 `PackageArtifact` 所有。
- `ServiceContract` 无代码、可先于实现发布，是 consumer compile 的唯一 service 协议输入。
- `ServiceDeployment`无源码，只把contract operation与external gateway绑定到typed package callable；
  不包含业务配置值或数据库/state binding。
- `RuntimeAssembly` 解析完整 deployment/package 闭包；第一版所有 service edge 都是
  `InProcessBoundary`。
- `RuntimeConfigSnapshot`是activation operational input，不是第五种代码artifact；committed generation
  并列钉住assembly/snapshot refs。
- `Publication` 不是领域对象或 compiler pipeline；`publish` 只保留为 registry 操作。
- package direct call 使用 Local ABI，可共享 heap、alias 和 mutation；service call 始终执行 boundary
  materialization 与 ActivationContext owner 切换。

## 2. 终态切换约束

不建立迁移 adapter ledger。一个生产域在所属阶段一次性切到终态，旧 owner 直接退出；
下游未完成时允许临时断链，不用兼容代码接回。

| 阶段 | 当阶段必须落地的终态 | 允许的临时状态 |
| --- | --- | --- |
| Phase 02 | compiler 只产出 `PackageArtifact` / `ServiceContract` | service CLI/watch/runtime 可暂时无法消费；不输出 `PackageUnit` / `ServiceUnit` / `serviceAssembly` |
| Phase 03 | 只用 `ServiceDeployment` / `RuntimeAssembly` 表达部署、完整闭包与 runtime admission | 尚未执行 service boundary；不保留旧 assembly/loader 兼容输入 |
| Phase 04 | `ActivationContext` / `InProcessBoundary` 是唯一 service 执行路径 | tooling 与业务服务尚未切换；不保留 remote relay fallback |
| Phase 05 | Skiff tooling、`skiff-packages` 与 `internals` consumer 全部消费四对象 | 无旧 reader/writer、artifact 转换、双轨或跨仓库 fallback |

## 3. Phase 01–02 经验与 Phase 02 后重审

Phase 01 已建立 canonical identity、type closure、typed effect leaf、PackageUnit projection 和跨层引用验证。
其早期独立任务并发有效，但后期形成长串行链；单个跨层验证提交触及 89 个文件，证明“一个 Agent 横跨
compiler/runtime/router/scripts”不是可接受任务边界。

Phase 02 已在 `main` 的 merge commit `629e78d` 完成 terminal compile-plane。它进一步证明：任务数量和
artifact 数量不是目标；只有扩大 DAG 宽度、缩短关键路径或隔离写入 owner 的拆分才有价值。真实的
producer/consumer 依赖不能伪并行，但也不应仅因可独立验证就机械增加阶段、完整 gate 与 merge。

后续阶段必须遵守：

- 共享 owner/API 是阶段内部的短 checkpoint，不单独占一个长期阶段。
- checkpoint 后按写入域拆 consumer，最多同时运行三个开发 Agent。
- 每阶段默认不超过三个实现波次；若超过，先检查是否遗漏可提前冻结的接口或错误制造了串行依赖。
- 阶段边界选择可独立验收的控制面或执行面，不再机械地为每个 artifact 对象单设阶段。
- 不为维持阶段间可运行性建立 compatibility/legacy bridge；旧路径在其 owner 被终态
  取代的阶段直接删除。

对 Phase 02 之后的旧三阶段划分重审后，结论是保留三个阶段边界，但修正阶段内部 DAG：

1. Phase 03 仍同时交付 `ServiceDeployment` 与 `RuntimeAssembly`，因为单独合入 deployment 不能解除任何
   跨阶段 consumer，也没有独立运行价值。运行时数据流仍严格是
   `deployment -> assembly resolution -> loader/linker admission`；实现 DAG 则先冻结两对象 schema/validator，
   再让 deployment projection、assembly resolver 与 typed runtime consumer 基于同一 checkpoint/fixture
   并行，最终按真实数据流合流。deployment mismatch 与 assembly closure/provider mismatch 分别做高风险
   verdict（可由同一只读 reviewer在同一 checkpoint分别输出），但只在最终稳定 assembly候选上运行一次
   阶段 gate、独立阶段验收并合入 `main`。
2. Phase 04 继续独立承接执行面。ordinary/error、async/stream/cancel、callback/native lane 共用
   owner/context/materialization kernel；先建立短 kernel checkpoint，再扇出 lane，最终统一
   ingress/internal dispatcher。把 kernel 单独升成阶段只会留下非终态 binding ABI。
3. Phase 05 继续同时完成 Skiff 本仓 tooling 与外部 ecosystem cutover，但不能从第一波三仓并行。
   先冻结 authoring/storage/control checkpoint，再迁移 Skiff registry/CLI/watch/router/test-runner，随后才从
   精确 integration checkpoint 扇出 `skiff-packages` 与 `internals`，最后建立一个跨仓稳定候选和唯一最终
   gate。外部 worktree 不要求此前先把中间 checkpoint 合入 `main`。

因此本次重审不增加阶段数。每新增一个阶段都会增加 integration worktree、计划与独立评审、稳定周期、
昂贵 gate、阶段验收和一次 `main` merge；没有新增 DAG 宽度或可消费稳定边界时，这些都是纯关键路径成本。

Phase 01、Phase 02 的 `phase-plan.md`、`phase-result.md` 与 task 文件是完成时的执行记录；其中指向未来阶段的
旧编号只表示历史 ledger。Phase 02 之后的当前编号与 owner 以本文和对应 outline 为准。

## 4. 当前阶段划分

```text
Phase 01  Canonical semantic / identity foundation（已完成）
    │
Phase 02  Compile plane：PackageArtifact + ServiceContract（已完成）
    │
Phase 03  Deployment and assembly plane：ServiceDeployment + RuntimeAssembly（已完成）
    │
Phase 04  In-process execution plane：ActivationContext + InProcessBoundary
    │
Phase 05  Ecosystem cutover：tooling、services 与 legacy deletion
```

### Phase 01：Canonical semantic / identity foundation

收敛 canonical JSON/framing、nominal/callable identity、package identity preimage、nominal type closure、
typed effect leaf、PackageUnit builder 和跨层 artifact reference validation。该阶段已验收并合入 `main`。

### Phase 02：Compile plane

阶段已完成并合入 `main`。最终 `PackageArtifact` 与独立 `ServiceContract` 共享 canonical boundary leaf，
provider 和 consumer 可只凭 contract 独立编译，consumer artifact 只保存 `ServiceCallRef`；compiler 不再有
publication/package/service 共同 source pipeline，也不产出任何旧 runtime DTO。最终证据见
`phase-02-compile-plane/phase-result.md`。

### Phase 03：Deployment and assembly plane

阶段已完成并通过独立验收。先完成无源码 `ServiceDeployment` 的 schema、identity、reference、projection 与
fail-closed validation，形成
高风险实现检查点；再从 root deployments 解析唯一 provider、完整 package/service closure、AssemblyIdentity、
package link image和per-ActivationContext service binding templates；最后让runtime loader/linker只消费
`RuntimeAssembly` 并完成 admission。schema/validator checkpoint 后，三个写入域可使用 canonical typed
fixtures 并行开发；集成与验收仍按真实 producer/consumer 顺序。阶段完成后 runtime production path 不读取
`ServiceUnit`、`PackageUnit`、`serviceAssembly` 或 adapter shape，但还不执行 service boundary。最终证据见
`phase-03-deployment-assembly/phase-result.md`。

2026-07-30 current-semantics correction：Phase 03当时的config/secret/state templates是历史实现，不再是终态。
Phase 05 F446删除这些字段；RuntimeAssembly只保留service binding template，配置由独立snapshot提供，service
DB由平台identity派生。

### Phase 04：In-process execution plane

建立 ActivationContext、service binding vector 和 transport-neutral materialization kernel；随后并行完成
ordinary/error、async/stream/cancel、callback/native capability 三类 lane。Ingress 与内部 service call
切到同一 dispatcher，package direct call 继续保留 same-heap mutation；所有 production service edge 都是
`InProcessBoundary`，production remote selection/fallback 不可达。

### Phase 05：Ecosystem cutover

先冻结不改变四对象 owner 的 authoring/storage/control API；再并行迁移 Skiff 本仓 registry/release、
CLI/watch/dev sync、router/runtime reload、test-runner 与 fixtures；本仓 checkpoint 稳定后，才并行迁移
`skiff-packages`、`internals` consumer 与实际 services。所有 consumer 直接读写四对象，不通过旧 artifact
adapter；最后删除跨仓 production legacy 路径，完成完整非 live verify、必要 live/chat smoke 与多 replica
验收。阶段只建立一次最终稳定候选、昂贵 gate 和独立验收。

F446在Phase 05内增加独立`RuntimeConfigSnapshot` operational path：三层service配置按Package ID分区，
generation并列钉assembly/snapshot refs；同时删除SecretRef、manifest/deployment state binding和无效
profile policy，并把数据库改为每service一个平台派生identity。它不增加代码artifact种类。

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
最终级：Phase 05 运行完整非 live verify、跨仓库 live/smoke、多 replica 验收与全局 legacy 搜索
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
