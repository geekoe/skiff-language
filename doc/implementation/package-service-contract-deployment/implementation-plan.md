# Package、Service Contract 与 Deployment 总体实现计划

状态：active；Phase 01 已细化，其余阶段只冻结边界

本文把 `doc/architecture/package-service-contract-deployment.md` 落成可逐阶段验收的实现路线。每个
阶段只向 `main` 合并一次；上一阶段验收后，下一阶段才展开任务 DAG，允许根据实现事实调整后续阶段
数量和范围，但不得绕过架构不变量。

## 1. 最终结果

最终生产数据流只有四个一等对象。Contract 可以先于任何实现独立产生，package 编译只消费所需
contract；它不是由 package artifact 派生：

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
- `ServiceContract` 是无代码、可先于实现发布的 typed protocol。
- `ServiceDeployment` 不读取源码，只把 contract operation 显式绑定到已编译 package callable，并
  绑定 config/state/resource owner。
- `RuntimeAssembly` 解析完整 deployment/package 闭包；第一版所有 service edge 都是
  `InProcessBoundary`。
- `Publication` 不是领域对象或 compiler pipeline；`publish` 只保留为 registry 操作。
- package direct call 使用 Local ABI，可共享 heap、alias 和 mutation；service call 始终保持 boundary
  materialization 与 ActivationContext owner 切换。

## 2. 分阶段迁移约束

仓库当前仍以 `PublicationInput`、`CompiledPublication`、`PublicationAbiUnit`、`ServiceUnit` 和
`serviceAssembly` 组织主要路径。不能在第一阶段机械全局重命名：那会同时修改 source compile、
contract、deployment、runtime 和 router，重新形成一个不可验收的大爆炸提交。

允许的阶段性办法只有薄 adapter：新 canonical owner 先接管规则，旧 aggregate 暂时消费结果。旧
aggregate 不得继续拥有或重新实现规则，并在对应阶段删除：

| 临时对象或路径 | 允许保留到 | 约束 |
| --- | --- | --- |
| `PublicationInput` / `CompiledPublication` / `LoweredPublication` | Phase 02 | 不得新增 service-only code analysis |
| `PublicationAbiUnit` 共同 aggregate | Phase 03 | Phase 01 后 identity/builder 只能委托 canonical leaf owner |
| code-owning `ServiceUnit` / service source compile | Phase 04 | 不得成为新 package/contract 事实 owner |
| 当前 `serviceAssembly` 作为 runtime/linker 语义 owner | Phase 05 | Phase 05 后只允许受结构 gate 约束的 tooling input adapter；不得再拥有 closure/link 语义，Phase 07 物理删除 |
| 当前 remote relay 的 production selection/fallback | Phase 06 | Phase 06 后 production 不可达；旧实现与 fixtures 在 Phase 07 物理删除 |
| 旧 registry、CLI、watch、test-runner 入口 | Phase 07 | 不得形成 dual-read/dual-write |

每项 adapter 都必须有结构 gate 防止新增调用点。这里的保留是仓库内分阶段施工，不是对已发布格式的
兼容承诺；artifact 与 fixture 可以在任何阶段直接重建。

## 3. 营地原则盘点

当前改动路径上已经确认的前置问题：

- `artifact-identity/src/lib.rs` 同时负责 File IR、package、publication、operation、service、runtime
  program 和 package-test identity，文件过长且 checker 反而把 owner 固定为单文件。
- nominal/type/method identity 分别使用 artifact-identity canonical JSON、artifact-model framed bytes
  和字符串拼接，存在三个算法 owner。
- package build/ABI identity preimage 由 serde 结构偶然决定，遗漏 `abiIdentityProjection`、
  `recoverableMetadata` 和 effect facts，却可能吞入 storage path/provenance。
- package artifact 在 production projection 与 package-test emission 各有一套 builder。
- boundary 与 recoverable 路径重复实现 nominal type closure、trace 和部分 schema validation。
- effect 在 source、projection、artifact 三层仍是 `Empty`、placeholder 或 raw metadata，无法表达
  “尚未分析必须保守拒绝”。
- service assembly identity、unit pointer 校验和 canonical path/ID 检查在 compiler、runtime、router
  与 scripts 间分散；现有 single-source checker 没覆盖所有生产消费者。

Phase 01 只清理上述会被后续直接放大的区域。ID authoring UX、router 其它历史问题和完整 effect
fixed-point 分析不在第一阶段顺手重构。

## 4. 阶段划分

```text
Phase 01  Canonical semantic / identity kernel
    │
Phase 02  PackageArtifact + package-only compiler + boundary eligibility
    │
Phase 03  ServiceContract + ContractRequirement / ServiceCallRef
    │
Phase 04  ServiceDeployment projection and validation
    │
Phase 05  RuntimeAssembly resolution and linking
    │
Phase 06  InProcessBoundary execution
    │
Phase 07  Tooling cutover and legacy deletion
```

### Phase 01：Canonical semantic / identity kernel

收敛 canonical JSON/framing、nominal/callable identity、package identity preimage、nominal type closure、
typed effect placeholder、PackageUnit builder 和跨层 artifact reference 校验。当前运行语义保持不变；
旧 publication/service aggregate 只能作为受控消费者。

### Phase 02：PackageArtifact 与唯一代码编译线

所有用户源码走 `PackageCompileInput -> PackageSourceModel -> LoweredPackage -> CompiledPackage`；生成
最终 `PackageArtifact`、`PackageLocalAbi`、callable semantic facts、sound may-effect/provenance 和
显式 `BoundaryCallableProjection`。删除 production `PublicationKind` 和 service-only code analysis。

### Phase 03：ServiceContract 与依赖编译

确定 code-free contract authoring 输入，生成独立 `ServiceContract`、`ContractTypeId`、closed schema、
`ServiceProtocolIdentity`；package 通过 `ContractRequirement` 编译，实际调用 lowering 为
`ServiceRequirement + ServiceCallRef`，不依赖 provider package。

### Phase 04：ServiceDeployment

确定 source-free deployment 输入；仅凭 typed PackageArtifact closure 与 ServiceContract 完成 operation、
effect、config/state/resource 绑定校验并生成 `ServiceDeployment`。删除 service source compile 和
code-owning `ServiceUnit`。

### Phase 05：RuntimeAssembly

从 root deployments 解析唯一 provider 和完整 package closure，代码在 replica 内只链接一次；生成
per-ActivationContext binding vector、activation templates 与 AssemblyIdentity。缺失、多 provider 或
remote-only closure 均 fail closed。旧 `serviceAssembly` 在本阶段退出 runtime/linker semantic owner；
若工具链尚未切换，只能保留无语义、受结构 gate 约束的 input adapter。

### Phase 06：InProcessBoundary

实现 detached materialization、provider owner 切换、async/stream/cancel/error/callback context 传播和
capability lifetime。Ingress 与内部 service call 使用同一 contract/binding 路径；当前 production
remote selection/fallback 变为不可达。旧 relay 代码与 fixtures 的物理删除留给 Phase 07。

### Phase 07：工具链切换与旧模型删除

registry/release、router/runtime reload、CLI/watch、test-runner、fixtures 与实际 services 全部切换；
物理删除旧 Publication aggregate、`serviceAssembly` tooling adapter、service relay、legacy adapters 和
旧 artifact readers，完成端到端与多 replica 验收。

## 5. Worktree 与提交协议

- 每阶段从最新 `main` 创建 `codex/package-service-phase-NN` 和同名 integration worktree。
- 当前阶段文档先在 integration branch 提交，所有 task worktree 从该提交或后续明确 checkpoint 创建。
- 并行任务必须声明非重叠写入范围；有依赖的任务从包含前置提交的新 checkpoint 创建，不能自行拼接
  多个任务分支。
- task Agent 完成后提交；主 Agent 按 DAG 合并进 integration branch并清理 task worktree/branch。
- 阶段验收通过后，以一个 merge commit 合并到 `main`。merge 后只做证据有效性核对，不机械重跑已在
  相同 commit 上通过的昂贵 gate。
- 每阶段 merge 是最小回滚单位；不通过长期兼容 shim 回滚。

## 6. 验证层级

```text
任务级：format / 静态检查 / 直接 crate 或 test filter
批次级：受影响 crate 组合 + 结构反向搜索
阶段级：对应 subject selector + 架构 gate + 独立只读验收
最终级：Phase 07 才运行完整非 live verify 和必要 live/smoke
```

删除或重写测试时必须说明旧测试锁定的语义，并提供 replacement test 或证明该语义已被整体删除。
任务 Agent 不跑全量套件替代影响分析；阶段 gate 对最终 commit 只运行一次。

## 7. 阶段调整规则

- 当前阶段发现会被本功能放大的重复或隐式契约：新增独立前置任务，更新 DAG 后继续。
- 只影响后续阶段：记录到下一阶段 overview，当前阶段不提前实现。
- 会改变四对象边界、两类调用语义或本地 assembly 方向：停止并请求用户决策。
- 评审只阻塞架构矛盾、不可执行 DAG、缺失 owner/删除条件或无法验收的问题；命名偏好、未来 remote
  细节和额外测试建议不阻塞。
