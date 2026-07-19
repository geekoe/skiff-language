# Phase 03：Deployment And Assembly Plane 实现计划

状态：planning；决策缺口审计完成，等待独立文档评审后执行

权威设计输入：`doc/architecture/package-service-contract-deployment.md`，重点 §2、§5、§9、§10、§12、
§14。本文只冻结 Phase 03 的实现 DAG、V1 内部表示、写入 ownership 与验收证据，不定义 authoring、
registry 或执行语义。

## 1. 阶段完成态

阶段验收时必须同时成立：

1. `ServiceDeploymentInput + ServiceContract + PackageArtifact closure -> ServiceDeployment` 是独立、无源码、
   source-free typed pipeline；不依赖 compiler AST/source/lowering/File IR signature 反推。
2. `roots + ServiceDeployments + ServiceContracts + PackageArtifact closure -> RuntimeAssembly` 完成 service/package
   cycle closure、唯一 provider、exact package build/local ABI、global ingress 与全部 binding/template 校验。
3. runtime loader/linker/admission 只消费 typed `RuntimeAssembly` 及其 exact refs；package code 按
   `PackageBuildId` 每 replica 只链接一次，activation-owned binding/config/state template 不共享 mutable owner。
4. runtime 以完整 assembly 建立候选并原子替换；admission 失败保留旧 active assembly，请求路径不 lazy-load
   artifact。
5. runtime production 新路径不读取 `ServiceUnit`、`PackageUnit`、raw `serviceAssembly`、source path、display
   name 或 raw JSON 来猜 target；不新增 adapter、dual-read、fallback 或 RemoteBoundary。
6. Phase 03 只落 ActivationContext template、service binding template 与 canonical link plan；不实例化/传播
   ActivationContext，不执行 materialization/dispatcher/callback/cancel，这些属于 Phase 04。

## 2. 已闭合的 V1 实现选择

这些是设计留给实现的内部选择，不新增产品语义：

- 新 top-level `deployment/` crate（`skiff-deployment`）拥有 deployment projection 与 assembly resolver；
  `artifact-model` 只拥有 DTO/leaf，`artifact-identity` 只拥有 canonical identity/validation，compiler 不依赖
  deployment。
- `DeploymentRevision` 是显式 opaque coordinate；`DeploymentArtifactIdentity` 与 `AssemblyIdentity` 是
  canonical content identity。identity preimage 排除 declared identity、path、diagnostic/display、resolved
  secret bytes 与 replica/host state；包含 revision、exact refs、operation/dependency/ingress/config/secret-ref/
  state/resource/policy、resolved graph、link plan 和 templates。
- package binding key 是 `(callerPackageBuildId, requirementAlias)`，value 是 exact `PackageArtifactRef`。
  同一 caller build 的 direct-link edge 在所有 activation 中必须相同；不同 build 可各自选择依赖。
- service selector key 是 `(callerPackageBuildId, serviceRequirementSlot)`，selector 只含 requirement 已有的
  `serviceId + contractVersion + expectedProtocolIdentity`。`RuntimeAssembly` 在当前 root/candidate set 中解析
  exact provider，并把结果放入 activation-relative template；consumer deployment 不钉 provider revision。
- secret 只保存 opaque binding reference。普通 config literal 与 secret ref 进入 deployment identity；secret
  backend 在同一 ref 后的内容轮换不进入 artifact/identity。
- canonical empty assembly 合法：空 roots/closure/link plan/templates/ingress，稳定 AssemblyIdentity；runtime
  admission 成功但任何 lookup 都 fail closed。
- artifact 保存 deterministic link plan；runtime `linked-program` hydrate 成含 `Arc`、loaded File IR/resource
  view 的共享内存 image。现有 service-specific `LinkedProgramImage` 不能原样成为 assembly wire。
- semantic refs 不携带 artifact filesystem path；storage/pointer/path trust boundary留给 Phase 05。

## 3. Canonical typed checkpoint

Wave 1 首先冻结以下 shared surface，后续 consumer 不得自行扩字段：

```text
PackageArtifactRef
  packageId / packageVersion / packageBuildId / packageLocalAbiIdentity

ServiceContractRef
  serviceId / contractVersion / serviceProtocolIdentity

ServiceDeploymentRef
  serviceId / contractVersion / deploymentRevision / deploymentArtifactIdentity

ServiceDeployment
  contract ref + implementation ref
  operation bindings: ContractOperationId -> PackageCallableId
  package bindings: (caller PackageBuildId, requirement alias) -> exact PackageArtifactRef
  service selectors: (caller PackageBuildId, service slot) -> exact contract selector
  ingress: typed external selector -> ContractOperationId
  config literals / secret refs / state / resource / runtime capability bindings
  timeout / activation policy
  deployment revision / artifact identity

RuntimeAssembly
  roots / resolved deployment refs / resolved contract refs / resolved package refs
  canonical package link plan
  service binding templates by activation
  activation config/state/resource templates
  global ingress bindings
  assembly identity
```

`ServiceContract` 仍是 operation descriptor/value plan/schema 的唯一 owner；assembly/deployment 只保存 typed
ref或精确 operation id，不复制第二份 descriptor table。所有 map/list 在 identity 前 canonical normalize；strict
wire 使用 deny-unknown。

## 4. 三波 DAG

```text
Wave 1
  T01 canonical deployment/assembly contract checkpoint

Wave 2（T01 合流后填满三个 worker）
  T02 source-free ServiceDeployment projection ──► T03 RuntimeAssembly resolver / binding templates
  T04 typed RuntimeAssembly loader / hydration
  T05 shared PackageArtifact linked-image model

Wave 3
  T06 assembly linker checkpoint（T03 + T04 + T05）
    ├── T07 whole-assembly host admission / atomic swap
    └── T08 downstream compile seam / terminal structure gates
  T09 stable-candidate integration gate
  A01 independent stage acceptance
```

运行时数据流仍是 deployment → assembly → load/link/admit。Wave 2 先以 T02/T04/T05 填满三个 worker；T02
合流后由释放的 worker从新 integration checkpoint执行 T03。并行来自 T01 冻结的 strict typed fixture/API，
而不是复制 producer规则；真实链路只在 integration branch合成一次。

## 5. 任务索引

| ID | 任务 | 依赖 | 风险 / 验收组 |
| --- | --- | --- | --- |
| D01 | [Independent phase-plan review](tasks/P3-D01-phase-plan-review.md) | Phase 03 文档 checkpoint | 只读；执行前 gate |
| T01 | [Canonical deployment/assembly contract](tasks/P3-T01-canonical-deployment-assembly-contract.md) | 文档评审 PASS | 高；独立 schema/identity checkpoint |
| T02 | [Source-free ServiceDeployment projection](tasks/P3-T02-service-deployment-projection.md) | T01 | 高；deployment batch |
| T03 | [RuntimeAssembly resolver](tasks/P3-T03-runtime-assembly-resolver.md) | T01、T02 API | 高；assembly batch |
| T04 | [Typed RuntimeAssembly loader](tasks/P3-T04-typed-runtime-assembly-loader.md) | T01 | 高；runtime loader batch |
| T05 | [Shared PackageArtifact linked image](tasks/P3-T05-shared-package-linked-image.md) | T01 | 高；linked-program batch |
| T06 | [RuntimeAssembly linker checkpoint](tasks/P3-T06-runtime-assembly-linker.md) | T03–T05 | 高；linker batch |
| T07 | [Whole-assembly host admission](tasks/P3-T07-whole-assembly-admission.md) | T06 | 高；admission batch |
| T08 | [Terminal runtime seams and structure gates](tasks/P3-T08-terminal-runtime-seams.md) | T06 | 中高；runtime consumer/structure batch |
| T09 | [Phase integration gate](tasks/P3-T09-phase-integration.md) | T02–T08 | gate owner；唯一昂贵阶段 gate |
| A01 | [Independent stage acceptance](tasks/P3-A01-stage-acceptance.md) | T09 | 独立只读验收 |

## 6. 写入 ownership

- T01 独占 `artifact-model/**`、`artifact-identity/**`、root `Cargo.toml`/`Cargo.lock`、新 `deployment/` crate
  shell、Rust verify subject registry 与 identity checker。T02–T08 不修改 frozen model/identity/checker；发现
  缺字段或 validator 缺口必须回报 T01 checkpoint amendment。
- T02 独占 `deployment/src/projection/**`；T03 独占 `deployment/src/assembly/**`。二者不得读取 compiler
  source crate，且不得互相复制 contract/package validation。
- T04 独占 `runtime/loader/**`；T05 独占 `runtime/linked-program/**`；T06 独占 `runtime/linker/**`。三者通过
  T01 model 与各自 public API 交接，不交叉修改。
- T07 独占 `runtime/host/src/loader/**`、whole-assembly state/admission/health owner，以及
  `request_entry.rs` 的 request-time lazy-load 删除面；不实现 Phase 04 dispatcher。
- T08 独占受影响的 `runtime/{activation,eval,package-test,request}/**` compile seam 和新 runtime artifact
  boundary checker；只删除/断开旧入口或适配 frozen terminal types，不恢复旧 DTO producer。
- T09 只做 integration、机械 blocker 修复、gate 与结果记录；任何语义缺口退回原 owner。
- 已经很长的 `runtime/host/src/loader/runtime_config.rs`、`runtime/linker/src/linker.rs` 及其千行子模块不得继续
  接收新 assembly owner；新职责按 model/validation/resolution/admission 拆文件。

## 7. 最早风险探针

### Deployment checkpoint

- missing/duplicate/extra operation、unknown public path、Unavailable callable、descriptor/effect/ContractTypeId
  mismatch 与缺失 config/state/resource/runtime-capability binding 全部失败。
- human public path 只在 projection trust boundary 解析；artifact 只保存 `PackageCallableId`。
- 修改 implementation build、operation/dependency/ingress/config/secret-ref/state/resource/policy 必变 deployment
  identity；改 map insertion order、path、diagnostic/display 不变。
- production dependency graph中没有 compiler source/lowering/AST 或 legacy runtime DTO。

### Assembly checkpoint

- A↔B service cycle闭合；同 requirement 零/多 provider、protocol mismatch、remote-only closure失败。
- `(callerBuildId, packageAlias)` exact build/version/local ABI 唯一；同 caller edge 冲突失败。
- 两个 activation 共享同一 package build，只产生一个 code slot，但 service/config/state templates独立。
- 两个不同 package 的 slot 0 不冲突；每条 template key 必须含 activation与caller build。
- empty assembly identity/round-trip/admission稳定，所有业务 lookup失败；global ingress collision失败。

### Runtime admission checkpoint

- package direct call严格走 requirement alias → expected local ABI → exact PackageArtifact → callableLinks。
- service call保留 activation-relative slot，不全局 patch provider executable。
- tampered artifact/ref/File IR/resource/link plan/template在 admission 前失败；失败不替换 active assembly。
- request path不触发 artifact load；health 可观察 active/candidate AssemblyIdentity与最后 admission 状态。
- runtime production反向搜索中旧 DTO/raw JSON/display-name/source linking和lazy-load归零；checker自检可识别
  改名、移动、重复 owner 和 test-only 伪例外。

## 8. 验证计划

任务迭代只运行直接 crate/过滤测试、targeted rustfmt 与结构探针。T09 是最终稳定候选的唯一昂贵 gate owner：

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-runtime-crate-dag.mjs
node scripts/check-runtime-artifact-boundaries.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

`check-runtime-artifact-boundaries.mjs` 由 T08 新增，并必须有 self-test。最终 targeted rustfmt覆盖本阶段所有
Rust改动文件；若 full workspace rustfmt仍命中未改 baseline，只记录可逐字复现的旧文件，不扩大本阶段。

Phase 03 不运行 router、test-runner、telemetry、live、chat smoke 或跨仓库 gate；这些 consumer 尚未切换，
不能证明 assembly control plane完成。若共享类型使它们无法编译，T08只做删除/断开旧入口所需的 terminal
compile seam，不建立 adapter。

## 9. 稳定候选与验收

- T01 schema/identity checkpoint单独做一次只读边界验收；T02+T03合成一次 deployment/assembly batch验收；
  T04+T05+T06合成一次 runtime-link batch验收。T07+T08的 admission/structure探针合流后由 T09与 A01覆盖，
  不再增加重复验收。T09前不运行完整阶段 gate。
- 所有 production owner合流、无在途写入/设计问题、真实 typed E2E与结构探针通过后，才固定 stable
  candidate与 stability epoch。
- A01在 exact clean commit上只读验收四对象 owner、identity separation、provider/package closure、shared
  code/per-activation templates、whole-assembly admission、legacy/fallback删除与证据有效性。
- blocker修复只使受影响证据失效；所有相关 blocker合流后才建立新 stability epoch，不逐项重跑完整 gate。

## 10. 明确非目标与停止条件

非目标：YAML/CLI authoring、registry/release/pointer/path layout、router control/reload、test-runner、实际 service
迁移、ActivationContext execution、materialization、async/stream/callback/cancel、RemoteBoundary、service级独立
扩缩容。

以下任一情况立即暂停受影响 DAG 分支并升级：

- 需要给 `ServiceRequirement` 写入 provider package/build/deployment revision/route，或按 display name选择。
- 需要把 resolved secret bytes写入 artifact/identity，或让同一 package direct-link edge随 activation变化。
- 需要 remote provider、router fallback、partial closure、request-time lazy load 才能 admission。
- 需要 deployment读取 AST/source/lowering，或 runtime从 File IR opaque signature重建 contract descriptor。
- 需要改变四对象边界、Package direct call same-heap语义或 Service boundary语义。

纯字段布局、crate落点、error type、normalized map/vector、fixture/helper与内部 cache策略由主 Agent按本文边界
决定，不升级为用户决策。
