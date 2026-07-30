# Phase 03：Deployment And Assembly Plane

状态：complete；production candidate `bedcd032` 已通过 P3-A01 独立验收

> 2026-07-30 current-semantics correction：本页下方关于config literal、SecretRef、state binding、
> deployment policy和activation template的内容只记录Phase 03当时的历史完成态，不再是可执行需求。
> 当前模型由canonical架构§11和Phase 05 unified config/service DB hard cut取代：
> ServiceDeployment/RuntimeAssembly不含业务配置或state，activation generation并列钉住独立
> RuntimeConfigSnapshotRef，数据库由平台按service identity派生。

## 架构边界

- 权威条款：设计 §2、§5、§9、§10、§12、§14。
- 本阶段完成两条 source-free typed pipeline，并把结果接入 whole-assembly runtime admission：

  ```text
  ServiceDeploymentInput + ServiceContract + PackageArtifact closure
    -> ServiceDeployment

  roots + ServiceDeployments + ServiceContracts + PackageArtifact closure
    -> RuntimeAssembly
    -> load/link/admit
  ```

- runtime 数据流严格按 deployment、assembly、admission 排序；实现任务可在 canonical schema/validator
  checkpoint 后使用真实 typed fixtures 并行，不得复制 schema 或构造 adapter。
- 文件/YAML/CLI authoring、registry pointer、router reload/control、service boundary execution不在本阶段；
  Phase 05 consumer 可以暂时不可用。

## 已闭合的 V1 实现选择

- service dependency selector 只使用 requirement 已有的
  `serviceId + contractVersion + expectedProtocolIdentity`；具体 deployment 由 assembly 在当前 root/candidate
  set 中唯一解析，不把 deployment revision 写回 consumer requirement。
- package direct-link binding 以 `(callerPackageBuildId, requirementAlias)` 选择 exact
  `PackageBuildId`；同一 key 不得因 ActivationContext 不同而变化。
- config/secret/state binding曾作为Phase 03 artifact输入；该历史选择已由2026-07-30 hard cut废止。
- canonical 空 assembly 合法：零 roots、零 closure/image/templates/routes、稳定 identity；admission 后任何
  service/ingress lookup 仍 fail closed。
- artifact 中保存 deterministic canonical link plan；runtime loader/linker hydrate 成共享只读内存 image，
  不把 `Arc`、打开的资源或 host handle 固化为 artifact wire。

## 完成态

- `ServiceDeployment`、`RuntimeAssembly`、semantic refs、distinct identity、strict wire、canonical
  assign/validate 与 mutation matrix 只有一个 owner。
- deployment 显式把每个 `ContractOperationId` 映射到稳定 `PackageCallableId`；ingress 只绑定 contract
  operation；boundary descriptor/effect 与全部 implementation requirement 在 projection 时校验。
- assembly 解析 service/package cycle closure；每个 service requirement 恰好一个本地 provider；同一
  `PackageBuildId` code 每 replica 只链接一次；当前终态由activation snapshot为每个deployment提供隔离
  Package-scoped ConfigView，service DB handle同样按deployment隔离。
- runtime 以整个 `RuntimeAssembly` 建立候选、load/link/admit 并原子替换；请求路径不再 lazy-load artifact。
- active assembly保留 exact immutable ServiceContract store；Phase 04可按 contract ref + operation ID读取 canonical
  descriptor/value plan，不从 template复制或在请求时重载。
- runtime production loader/linker/admission 不读取 `ServiceUnit`、`PackageUnit`、raw `serviceAssembly`、
  display name、source path 或 provider executable guess；不生成 dual path、fallback 或 compatibility adapter。
- 本阶段只形成 ActivationContext template 与 `InProcessBoundary` binding plan，不实例化/执行 boundary；
  async/stream/callback/cancel 传播和 dispatcher 属于 Phase 04。

## 三个实现波次

1. canonical deployment/assembly schema、identity、reference、validator与新 crate shell checkpoint；独立验收
   PASS后扇出。
2. deployment projection、assembly resolver、typed loader、shared package image四个同层 owner按三个 worker动态
   排队；projection与resolver分别消费 frozen DTO/fixture，不互相串行。两个 control-plane verdict PASS且
   loader/image完成后建立 linker checkpoint，再做 runtime-link验收。
3. runtime-link PASS后，host whole-assembly admission、下游 consumer seam、结构 checker三个 owner并行收敛；
   建立稳定候选、
   运行唯一阶段 gate 和独立验收。

## 阶段验收

- 真实 `contract + provider/consumer PackageArtifact -> deployment -> assembly -> load/link/admit` 路径通过，
  不读取 AST、source text、lowering helper 或旧 runtime DTO。
- missing/duplicate/extra operation、Unavailable callable、descriptor/effect/ContractTypeId mismatch、
  package build/version/local-ABI/capability mismatch全部fail closed；config snapshot验证不属于deployment。
- A↔B service cycle 可闭合；零/多 provider、remote-only closure、binding slot 越界/重复、ingress collision、
  tampered artifact/ref/link plan 全部在 admission 前失败。
- 两个 activation 可复用同一 package code，并为相同 `(callerPackageBuildId, slot)` 绑定不同本地 provider；
  ConfigView/service DB handle/callback mutable owner不因共享code而共享。
- 空 assembly admission 成功但没有任何 dispatch target；admission 失败不改变当前 active assembly。
- admission后 canonical contract descriptor/value plan仍可 typed lookup，不产生第二 descriptor owner或请求时I/O。
- 结构反向搜索与 checker self-test 证明 runtime production 新路径没有旧 DTO、raw JSON linking、
  request-time lazy load 或第二 identity/schema owner。
