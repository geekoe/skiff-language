# Phase 02：Compile Plane 实现计划

状态：active；2026-07-18 从 `9ca2547` terminal-only checkpoint 重建

权威设计输入：`doc/architecture/package-service-contract-deployment.md`。本文只冻结 Phase 02 的实现选择、
DAG、写入 ownership 和验收证据，不改变四对象模型。

## 1. 阶段完成态

阶段验收时必须同时成立：

1. `ServiceContractDefinition -> ServiceContract` 是无 provider code 的独立 typed pipeline；本阶段提供
   Rust typed API 和 fixture，文件/CLI authoring 拼写留给 Phase 05。
2. `PackageCompileInput -> PackageSourceModel -> LoweredPackage -> CompiledPackage -> PackageArtifact` 是
   用户源码唯一 production compile pipeline；不存在 package/service kind 分支和共同 publication bundle。
3. `PackageArtifact` 独立拥有 PackageLocalAbi、implementation links、requirements、callable semantic facts、
   boundary projections 和 unresolved ServiceCallRefs；不嵌入 `PublicationAbiUnit` 或 `ServiceUnit`。
4. `ServiceContract` 独立拥有 service coordinate、protocol identity、operations 与 closed boundary schema；
   不含 provider/build/deployment/route/config/runtime 字段。
5. provider 与 consumer 只凭同一 contract 分别编译；consumer 不读取 provider package、ServiceUnit、
   serviceAssembly 或 deployment。
6. 每个可能被 deployment 选择的 package API callable 都有 Local ABI 和显式
   `BoundaryCallableProjection::{Available, Unavailable}`。Unknown/不支持行为保守 Unavailable，但 callable
   仍可被 package direct call。
7. compiler 完成 sound conservative may-effect/provenance fixed point；简单可证明的 detached-data callable
   可以 Available，复杂或未知行为不能被误判安全。
8. 实际 service call site 生成稳定 binding slot、`ServiceRequirement` 与
   `ServiceCallRef { serviceRequirementSlot, contractOperationId, expectedProtocolIdentity }`；只声明未调用不
   产生 runtime requirement。
9. compiler 不产出 `PublicationAbiUnit`、`PackageUnit`、`ServiceUnit`、`serviceAssembly` 或它们的
   pointer/manifest，不包含 legacy runtime adapter、compatibility checksum、dual-write 或 fallback。
10. 本阶段直接触碰的 publication/service compile owner、重复 identity/projection、超长新增逻辑均已
    删除或拆分；结构 gate 证明 compiler 生产路径只有两种终态 artifact。

## 2. 本阶段实现选择

架构明确不冻结 contract authoring 文件和 CLI。本阶段选择最小 typed 边界，避免提前发明用户语法：

- 新增严格 typed `ServiceContractDefinition` Rust 输入，可由测试、未来 CLI 或平台 producer 构造。
- Phase 02 不新增 `contract.yml`、IDL 或源码 declaration 语法；Phase 05 再为 typed API 接 authoring UX。
- 现有 service CLI/watch/runtime 在后续阶段完成前可以不可用。本阶段不从 provider source
  反推 contract/binding，不为保持可运行性保留旧 service publication 入口。
- `ContractTypeId` 是独立 tagged identity，不复用或改名 `AbiTypeId`。artifact type ref 以显式 contract
  symbol/reference 表达，schema closure 通过 ContractTypeId 查找，不靠 display string 猜测。
- contract operation identity 由 service coordinate、contract version 和稳定 operation key 派生；
  ServiceProtocolIdentity 由完整 operation descriptor 与 closed schema canonical projection 派生。
- provider/build/deployment/route、`BoundaryImplementationRequirements` 和诊断文本不进入 protocol identity。
- `BoundaryOperationDescriptor` 的 error/stream/cancel/callback/value-plan/effect guarantee 字段全部显式；当前
  语言暂不支持的 lane 使用 tagged unavailable/unsupported 状态，不能通过字段缺失表达。
- PackageArtifact 的 Available projection只保存contract-agnostic `BoundaryOperationContract`；真实
  `BoundaryOperationDescriptor`、`ContractOperationId`和contract stable key只由ServiceContract拥有。禁止用
  PackageCallableId或public path伪造contract identity。
- `PackageArtifact` identity 继续复用 Phase 01 canonical framing，但以新显式 projection 重建并更新 marker/
  prefix/golden；不把旧 PackageUnit serde shape 当 preimage。

这些是设计文档允许实现阶段选择的 wire/API 细节；若实现发现无法在不改变调用语义的情况下表达，停止
受影响任务并请求用户决策。

## 3. Canonical owner

```text
artifact-model
  contract type / boundary descriptor / requirements / service call refs
  PackageArtifact / ServiceContract typed wire only

artifact-identity
  contract type / operation / protocol / package local+build identity
  assign + validate + mutation golden

compiler contract leaf
  ServiceContractDefinition -> closed ServiceContract

compiler source
  typed call-target/provenance + sound may-effect fixed point

compiler lowering
  ContractRequirement lookup + ServiceCallRef generation

compiler projection/emission
  PackageArtifact + BoundaryCallableProjection

compiler driver
  only PackageCompileInput pipeline + code-free ServiceContractDefinition pipeline
```

旧 `compiler/projection/src/contract/**` 中纯 schema/type-closure 算法可以抽出复用；含 implementation binding、
service config 或 runtime projection 的 aggregate 不得成为 ServiceContract owner。

## 4. DAG 与三个实现波次

```text
9ca2547 = T01 + T02 + T03 + T04 clean checkpoint
  ├── R03 exact canonical payload symbols（只移植 canonical patch）
  ├── R11 canonical contract schema fidelity（移植已验收 commit）
  └── T05 package-only central compiler terminal cutover（从干净基线重做）

R03 + R11
  └── T05A terminal PackageArtifact projection/emission handoff

T05 package-only dataflow checkpoint
  ├── T05 terminal driver/facade cutover
  └── T05B terminal compiler structure gates

R03 + R11 + T05 + T05A + T05B checkpoint
  ├── R04 canonical package config shape
  ├── R06 complete canonical package requirement closure
  └── R13 canonical package DB schema validation

R06
  └── T05C1 terminal compiler facade/input cleanup（与 R04/R13 并行）

R06 + R13
  └── T05C2 terminal compiler model cleanup（与 T05C1/T05C 并行）

T05C1 + T05C2
  ├── T05C core/orphan publication-ABI cleanup
  ├── T05C3 terminal source helper cleanup
  ├── T05C4 terminal lowering cleanup
  └── T05C5 terminal compile handoff repairs

R04 + R06 + R13 + T05C + T05C3 + T05C4 + T05C5
  └── R10 canonical compiler integration fixtures

R03 + R04 + R06 + R10 + R11 + R13
  └── T07 phase integration gate -> A01 independent acceptance
```

| 波次 | 并行任务 | 说明 |
| --- | --- | --- |
| checkpoint | R03、R11、T05、T05A、T05B | dataflow 后按 driver、projection/emission、structure gates 三域并行 |
| 1 | R04、R06、R13、T05C1、T05C2 | canonical wave 与 facade/input/model checkpoint cleanup |
| 2 | T05C、T05C3、T05C4、T05C5 | core/orphan、source、lowering 与两个窄 handoff repair 并行收尾 |
| 3 | R10 | production cleanup 合流并通过结构探针后，只迁移 canonical test fixtures |
| 4 | T07 | 唯一最终 compiler/foundation gate、结构审计和结果记录 |

T06/R02/R05/R07/R08/R09 与旧 R10B/R10C 位于被放弃的 integration tail，不进入新分支 ancestry；
对应终态能力在 Phase 03–05 直接实现，canonical fixture 部分由 R10 重建。R12 的“在污染 tree 上清理”
策略被干净基线重建吸收，不再执行。

R10 是 T07 首次编译 gate 暴露的独立前置：旧 compiler integration fixture 仍把
service 当作代码与部署聚合。它按真实测试语义拆成 canonical package fixture 与显式
ServiceContract fixture，不修改 production 语义。T07 只负责最终稳定候选的昂贵 gate、纯机械
fixture 和结果记录，不新增语义。A01 只读验收。

## 5. 任务索引

| ID | 任务 | 依赖 | 风险 / 验收组 |
| --- | --- | --- | --- |
| T01 | [Canonical compile-contract checkpoint](tasks/P2-T01-canonical-compile-contract.md) | 无 | 高；独立 checkpoint |
| T02 | [Sound effect/provenance analysis](tasks/P2-T02-effect-provenance.md) | T01 | 高；effect group |
| T03 | [Contract requirement 与 ServiceCallRef lowering](tasks/P2-T03-service-call-lowering.md) | T01 | 高；dependency group |
| T04 | [PackageArtifact 与 boundary projection](tasks/P2-T04-package-artifact.md) | T01 | 高；artifact group |
| T05 | [Package-only compiler terminal cutover](tasks/P2-T05-compiler-cutover.md) | `9ca2547` | 高；从干净基线重做 central compiler |
| T05A | [Terminal PackageArtifact projection/emission handoff](tasks/P2-T05A-terminal-package-artifact-handoff.md) | R03、R11 | 高；projection/emission 独占 owner |
| T05B | [Terminal compiler structure gates](tasks/P2-T05B-terminal-compiler-structure-gates.md) | T05 dataflow checkpoint | 中；scripts/checker 独占 owner |
| T05C1 | [Terminal compiler facade/input cleanup](tasks/P2-T05C1-terminal-compiler-facade-input-cleanup.md) | R06 | 高；与 R04/R13 并行的 checkpoint repair |
| T05C2 | [Terminal compiler model cleanup](tasks/P2-T05C2-terminal-compiler-model-cleanup.md) | R06、R13 | 高；input-model/source/lowering blocker repair |
| T05C | [Terminal compiler core/orphan cleanup](tasks/P2-T05C-terminal-compiler-production-cleanup.md) | T05C1、T05C2 | 高；core 与 orphan crate blocker repair |
| T05C3 | [Terminal source helper cleanup](tasks/P2-T05C3-terminal-source-helper-cleanup.md) | T05C2 | 中；source orphan owner cleanup |
| T05C4 | [Terminal lowering cleanup](tasks/P2-T05C4-terminal-lowering-cleanup.md) | T05C2 | 高；empty index/parameter-chain cleanup |
| T05C5 | [Terminal compile handoff repairs](tasks/P2-T05C5-terminal-compile-handoff-repairs.md) | T05C1、T05C2 | 中；input/projection 窄断链 repair |
| T06 | [Legacy runtime/test consumer adapter](tasks/P2-T06-legacy-consumers.md) | 已取消 | 不进入新 integration |
| R02 | [Explicit contract-operation route binding](tasks/P2-R02-contract-operation-route-binding.md) | 延后 Phase 03/04 | 不通过旧 runtime shell 落地 |
| R03 | [Exact canonical payload symbols](tasks/P2-R03-exact-canonical-payload-symbols.md) | `9ca2547` | 中；只移植 canonical patch |
| R04 | [Canonical package config shape](tasks/P2-R04-canonical-package-config-shape.md) | T05、R11 | 只实现 canonical requirements |
| R06 | [Complete canonical package requirement closure](tasks/P2-R06-canonical-package-requirement-closure.md) | T05、R03 | 高；canonical dependency 前置 |
| R05 | [Exact legacy package ABI witness](tasks/P2-R05-exact-legacy-package-abi-witness.md) | 已取消 | 不进入新 integration |
| R08 | [Dev router empty active set](tasks/P2-R08-dev-router-empty-active-set.md) | 延后 | 不是 Phase 02 验收依赖 |
| R09 | [Canonical test dependency closure](tasks/P2-R09-canonical-test-dependency-closure.md) | 已吸收 | canonical graph 进 R10；旧 holder 不移植 |
| R07 | [Service-test local entrypoint assembly](tasks/P2-R07-service-test-local-entrypoint.md) | 延后 Phase 03/04 | 不通过旧 runtime shell 落地 |
| R11 | [Canonical contract schema fidelity](tasks/P2-R11-canonical-contract-schema-fidelity.md) | `9ca2547` | 高；移植已验收 commit `834cd55` |
| R10 | [Canonical compiler integration fixtures](tasks/P2-R10-canonical-compiler-integration-fixtures.md) | T05C、T05C3、T05C4、T05C5、R03、R04、R06、R11、R13 | 中；canonical test architecture |
| R12 | [Terminal compile-plane cleanup](tasks/P2-R12-terminal-compile-plane-cleanup.md) | 已吸收 | 由 clean-base reconstruction 取代 |
| R13 | [Canonical package DB schema validation](tasks/P2-R13-canonical-package-db-schema-validation.md) | T05 | 中；package DB/schema owner |
| T07 | [Phase integration gate](tasks/P2-T07-phase-integration.md) | R03、R04、R06、R10、R11、R13 | gate owner |
| A01 | [Independent stage acceptance](tasks/P2-A01-stage-acceptance.md) | T07 | 独立只读验收 |

## 6. 写入冲突规则

- T01 独占 `artifact-model`、`artifact-identity`、新 contract leaf crate、root workspace/API policy，并在
  `compiler/source` 冻结波次 2 共用的 resolved call-target fact carrier 与 facade。波次 2 不修改公共 wire 或
  carrier shape；发现缺字段必须回报 T01 checkpoint amendment。
- T02 独占 T01 checkpoint 之后的全部 `compiler/source/**` 改动，包括 effect/provenance 算法、contract call
  target 的 source 侧填充、最小 facade export 与直接 source tests；不修改 input/lowering、projection、
  emission、driver。
- T03 独占 contract dependency reader、dependency operation index、service-call lowering/external refs及其
  tests；**不得修改 `compiler/source/**`**。它以 T01 冻结的 typed carrier/contract identity 为输入，缺字段
  回报 checkpoint owner，不在自己的分支扩展 source model。
- T04 独占新的 PackageArtifact projection/materialization、boundary projection 与直接 emission tests；不修改
  source/lowering/driver。
- T05 独占 `compiler/driver/pipeline`、input-model compile input、compiled/lowering/source 根类型、
  projection-input、compiler facade、旧 `service_publication_tests.rs` disposition，以及**全部** compiler structure
  checker 与 checker self-test。T05 只消费 `9ca2547` 的 T01–T04 API；禁止整体 cherry-pick 旧 T05
  `9adfd64` 或其后 integration tail；不得修改 `compiler/projection/**`、`compiler/emission/**`。
- T05A 独占 `compiler/projection/**`、`compiler/emission/**` terminal cutover与直接 tests；保留 R03 export
  payload 语义和 R11 schema leaf，不修改 T05 central 目录或 foundation artifact crates。
- T05B 独占 compiler structure checker、crate-DAG/public-API policy及 self-tests/fixtures；不修改 Rust
  production 或 tests。T05/T05A 不再修改 checker。
- T05C1 在 R06 后独占 terminal compiler facade/Cargo、input、projection-input 和直接
  error surface cleanup；不得修改 core/source/lowering/driver/emission/checker 或 integration tests。
- T05C2 在 R06/R13 后独占 input-model/source/lowering 旧 model cleanup、publication-ABI production edge
  与 orphan crate disposition；不得修改 facade/input/core/projection/emission/driver/checker 或 integration tests。
- T05C 在 T05C1/T05C2 后独占 core aggregate 与 orphan publication-ABI crate/gate-config cleanup；不得修改
  source/lowering 或 compiler integration tests。
- T05C3 独占 T05C2 后 source orphan helper cleanup；T05C4 独占 lowering empty index/参数链 cleanup；
  二者不得修改彼此或 core/facade/Cargo/checker/integration tests。
- T05C5 独占 package-only input origin/service reader 与 config requirement projection error窄修复；不得恢复
  已删除 enum/error variant，也不得修改其它 production owner。
- R03 独占 canonical package export link 中 payload symbol 的精确投影与直接测试；map key
  继续表达 public path，link `symbol` 只能表达 file/index 指向的真实 payload declaration。
- R04 独占 canonical package config requirements 与 `ConfigShape` 的唯一 typed 表达；不为旧
  adapter/presentation 生成 source provenance、uses 或 activation 第二 owner。
- R06 独占 canonical `PackageRequirement` 闭包与 File IR package ref 覆盖校验；编译器
  内建 std 只能从同一 canonical package graph 的 std `PackageArtifact` 获得精确 version/local ABI，
  不允许 adapter 特例、硬编码 identity 或第二次 compile。
- R10 独占 compiler test-support 与 `compiler/tests/**` 中的旧 service publication fixture 退役；
  不修改 `compiler/driver/service_publication_tests.rs`。源码、
  effect、logical DB schema 及 compile 语义测试改用 canonical package/test-support API；只有真正验证
  service protocol/conformance 的测试才构造显式 ServiceContract。禁止空/fake contract、provider 反推、
  一个新的万能聚合 builder，也不得为了通过编译整批删除 Cargo test targets。
- R11 独占 canonical ServiceContract schema grammar、normalization、validation 与 identity。
  discriminator/branch tag、map key identity 和当前 recursion policy 必须进入 typed contract；旧
  JSON-schema/serviceAssembly presentation 不进入 R11，也不得从 provider source 推导 contract。
- R12 不执行。新 integration 没有后半段兼容 ancestry；旧 compiler owner 由 T05 在 clean base 上直接
  终态替换，结构归零证据由 T07 验收。
- R13 独占 package source/DB schema 声明的 canonical 校验；不读 service id、collection mapping、
  Mongo namespace 或 deployment policy。
- 超过数百行的新 owner 要按 model/validation/projection/tests 拆文件；同一规则在两个模块出现即暂停并抽
  canonical leaf，不能以时间为由复制。

## 7. 验证计划

任务只运行自己的聚焦命令。最终稳定候选的唯一昂贵 gate owner 是 T07：

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

runtime/test-runner/router 尚未转向终态 artifact，不以它们的旧 service 功能测试作为 Phase 02
gate。但 workspace 与直接依赖 crate 必须能编译；由 T07 按最终 diff 选择最小 `cargo check`
集合并记录暂时断链。

结构 gate 至少证明 production canonical path 中：

```text
PublicationInput / PublicationKind / CompiledPublication / LoweredPublication = 0
PackageArtifact 或 ServiceContract 内 PublicationAbiUnit / ServiceUnit = 0
contract-only consumer path 的 provider build/deployment/route/executable target = 0
canonical File IR 的旧 ServiceDependencySymbol producer = 0
compiler production 中 PublicationAbiUnit / PackageUnit / ServiceUnit / serviceAssembly producer = 0
compiler production 中 legacy_runtime_adapter / compatibility / fallback allowlist = 0
```

## 8. 阶段验收样例

1. 无 provider source/artifact，仅凭 definition 生成、round-trip、assign/validate ServiceContract。
2. provider package只依赖 contract 即编译；显式 wrapper Available，本地 mutation/alias helper Unavailable 但
   保留 Local ABI。
3. consumer package只依赖 contract 即编译；真实调用生成一个稳定 slot、usedOperations 与 ServiceCallRef，
   artifact 不含 provider build/package/deployment/route/executable。
4. 未知 operation、protocol mismatch、schema不闭合、package-local nominal冒充ContractTypeId均失败。
5. direct package call 仍走 implementation link并保留 same-heap alias/mutation。
6. recursive SCC、跨package调用、write/return/throw alias、escape、callback/stream/spawn/DB/native/unknown
   effect保守传播；Unknown永不 Available。
7. 旧 service publication 入口不存在；没有显式 ServiceContract 时不推导 contract、不生成
   binding，也不写出任何 runtime artifact。
8. compiler test fixture 只保留 production PackageArtifact/ServiceContract 和 canonical package graph，
   不携带空 `runtime_units`、PackageUnit 或 ServiceUnit 槽位。

## 9. 明确非目标

- 不选择 provider、不生成 ServiceDeployment 或 RuntimeAssembly。
- 不执行 service call，不实现 ActivationContext 或 InProcessBoundary runtime。
- 不定义最终 contract YAML/IDL/CLI authoring UX。
- 不保证现有 service CLI/watch/runtime 可用；也不建立 legacy/compatibility adapter 保持它们可用。
- 不要求在 Phase 02 物理删除尚未触碰的 runtime/router 内部 DTO 定义，但 compiler
  不得产出或引用它们。
- 不追求完美 effect precision；只要求 sound、可证明简单路径可用、未知路径 fail closed。

## 10. 停止条件

- 需要把 provider package/build/deployment/route 写进 ContractRequirement、ServiceRequirement 或 ServiceCallRef。
- 为保持旧 CLI/runtime 可用而新增 adapter、旧 DTO producer、provider inference、dual-write 或 fallback。
- effect 分析以无法证明的 false 表示安全，或 Unknown 被解释为无 effect。
- PackageArtifact/ServiceContract 通过嵌入 PublicationAbiUnit/ServiceUnit 快速完成。
- T01/T05 后发现共享 checkpoint 缺少会改变架构语义的字段，而不是纯实现字段。

前三项直接判定方案错误；最后一项暂停受影响分支并请求用户决策。
