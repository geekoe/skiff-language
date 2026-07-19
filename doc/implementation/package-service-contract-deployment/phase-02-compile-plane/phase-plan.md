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
- package source复用现有dependency namespace：`payments.User`解析到validated contract中的
  `ContractTypeId`；dependency source call复用现有`/`地址语法，`payments/charge(...)`按同一contract的
  operation descriptor完成source typecheck。`.`只用于qualified type和address后的成员访问。
  package alias与contract alias冲突在compile-input trust boundary失败，不靠type/call上下文消歧。
- PackageArtifact callable signature必须沿source typed facts显式携带`PackageTypeRef::{Local, Contract,
  Container, Nullable}`；projection不得从File IR把全部类型重建为Local。当前没有source命名与终态
  `PackageTypeRef`表达的inline structural contract shape保守拒绝，不静默flatten。
- 用户选择方案A：File IR executable signature只保存execution type representation。Contract leaf固定投影为
  opaque builtin/native `unknown`，container/nullable递归保留；精确ContractTypeId只由source facts、
  PackageArtifact和ServiceContract持有。File IR不新增Contract variant，也不允许ServiceSymbol/display fallback。
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
  consume resolved ContractRequirement + ServiceRequirement / ServiceCallRef generation

compiler projection/emission
  PackageArtifact + BoundaryCallableProjection

compiler driver
  only PackageCompileInput pipeline + code-free ServiceContractDefinition pipeline
```

旧 `compiler/projection/src/contract/**` 中纯 schema/type-closure 算法可以抽出复用；含 implementation binding、
service config 或 runtime projection 的 aggregate 不得成为 ServiceContract owner。

## 4. DAG 与实现波次

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
  └── T05C5 terminal compile handoff repairs

T05C5 + canonical package-call decision A
  └── T05C6 canonical File IR package-call target checkpoint

T05C6
  ├── T05C4 terminal lowering cleanup
  ├── T05C7 package-call coverage
  ├── T05C8 package-call compiler consumers
  └── T05C9 File IR identity version

T05C4 + T05C8 + T05C9 checkpoint review
  ├── T05C10A type-only package requirement coverage
  ├── T05C10B package-call lowering fail-closed
  └── T05C10C canonical package-call reference validator checkpoint

T05C10C
  ├── T05C10D identity validator consumer
  └── T05C10E materialization validator consumer

T05C9 + terminal compiler cleanup
  └── T05C10F identity checker terminal-owner repair

T05C10C + T05C10D + T05C10E + T05C10F checkpoint acceptance
  └── T05C10G identity checker package-call owner coverage

R04 + R06 + R13 + T05C + T05C3 + T05C4 + T05C5 + T05C6 + T05C7 + T05C8 + T05C9 + T05C10A + T05C10B + T05C10D + T05C10E + T05C10F + T05C10G
  └── R10 canonical compiler shared fixture checkpoint

R10
  └── R10A shared-fixture lane contract probes

R10A
  ├── R10B type/import/File IR fixtures
  ├── R10C artifact/config/DB/resource fixtures
  ├── R10D contract/disposition fixtures
  └── R10E std/prelude schema fixtures

R10B + R10C + R10E checkpoint review
  └── R10G shared fixture file-write owner

R10G
  └── R10F std package imports fixture

R03 + R04 + R06 + R10B + R10C + R10D + R10E + R10F + R10G + R11 + R13
  └── T07 phase integration gate -> A01 independent acceptance

A01 hidden-owner finding
  └── T05C11 orphan publication owner cleanup

T05C11
  └── T05C12 terminal compiler public-shape gate

T05C12 + remaining A01 findings closed
  └── T07 evidence refresh -> A01 independent re-acceptance

A01 typed-contract finding + qualified alias decision
  ├── T03A canonical contract semantic facts
  └── R10H typed contract fixture checkpoint

T03A
  ├── T03B qualified contract type resolution
  └── T03D terminal service-call lowering

T03B
  ├── T03C contract call type checking
  └── T04A contract-aware callable signature handoff

T03C + T03D + T04A + R10H
  └── F09A initial R10I probe + production acceptance（6/7，FAIL historical evidence）

F09A（只作为证据，不是活动DAG节点）
  ├── wrong dot dependency-call surface -> T03E canonical dependency source address
  └── missing File IR contract execution representation + user decision A -> T03F source executable signature facts

T03E + T03F
  ├── T03G File IR execution type representation
  └── T04B signature handoff owner cleanup / evidence refresh

T03G + T04B
  ├── R10I resume provider/consumer contract E2E（7/7）
  └── F09B production re-acceptance（public-instance exact handoff FAIL historical evidence）

F09B（只作为证据，不是活动DAG节点）
  └── interface仍以ServiceSymbol做conformance/execution owner -> T03H exact interface signature facts

T03H
  ├── T03I interface File IR execution projection
  └── T04C compiled/projection public-instance owner cutover

T03I + T04C
  └── R10I evidence refresh + production re-acceptance
      └── T07 evidence refresh -> A01 independent re-acceptance
```

| 波次 | 并行任务 | 说明 |
| --- | --- | --- |
| checkpoint | R03、R11、T05、T05A、T05B | dataflow 后按 driver、projection/emission、structure gates 三域并行 |
| 1 | R04、R06、R13、T05C1、T05C2 | canonical wave 与 facade/input/model checkpoint cleanup |
| 2 | T05C、T05C3、T05C5、T05C6 | production cleanup后冻结canonical package-call schema checkpoint |
| 3 | T05C4、T05C7、T05C8、T05C9 | lowering、emission、driver/core、identity consumers并行迁移 |
| 4a | T05C10A、T05C10B、T05C10C | checkpoint review 暴露的 requirement、fail-closed、共享 validator 前置修复 |
| 4b | T05C10D、T05C10E、T05C10F | validator consumers 与 terminal identity checker 并行收敛 |
| 4c | T05C10G | 独立验收暴露的 package-call checker owner 漏管修复 |
| 5a | R10 | production cleanup合流后先冻结共享 canonical fixture API |
| 5b | R10A | 三条 consumer lane 的 representative compile-only probe，最后收敛 shared API |
| 5c | R10B、R10C、R10D、R10E | consumer 批次按可用三个 worker 槽动态扇出，文件 ownership 互斥 |
| 5d | R10G | independent review 暴露的共享 fixture file-write 抽象前置 |
| 5e | R10F | 修复遗漏的 std_package_imports terminal target |
| 6 | T07 | 唯一最终compiler/foundation gate、结构审计和结果记录 |
| 7a | T05C11 | A01 暴露的孤儿 publication aggregate/adapter 物理删除 |
| 7b | T05C12 | 对 compiled/projection-input terminal public shape 建结构 gate |
| 8a | T03A、R10H | canonical semantic facts与typed fixture入口并行 |
| 8b | T03B、T03D | qualified type resolution与terminal lowering并行 |
| 8c | T03C、T04A | contract call typing与exact signature handoff并行 |
| 8d | F09A | provider/consumer真实source初次probe为6/7 FAIL，形成后续前置finding |
| 8e | 未执行 | F09A阻断旧T07/A01，不产生无效gate证据 |
| 9a | T03E、T03F | `/` dependency address与all-executable exact source facts并行检查点 |
| 9b | T03G、T04B | File IR execution representation与signature handoff owner cleanup并行 |
| 9c | R10I、F09B production复验 | 真实source E2E 7/7；独立复验暴露interface/public-instance第二owner |
| 9d | T03H | 先冻结source exact interface/conformance query checkpoint |
| 9e | T03I、T04C | interface execution projection与compiled→projection public-instance cutover并行收敛 |
| 9f | R10I evidence refresh、production复验 | 只重跑被T03H/T03I/T04C失效的真实source与production证据 |
| 9g | T07 → A01 | 唯一最终gate后独立阶段验收 |

T06/R02/R05/R07/R08/R09 位于被放弃的 integration tail，不进入新分支 ancestry；对应终态能力在
Phase 03–05 直接实现。R12 的“在污染 tree 上清理”
策略被干净基线重建吸收，不再执行。

R10 是共享 fixture checkpoint；R10B/R10C/R10D 按真实测试语义并行消费 canonical package fixture 与
显式 ServiceContract fixture，不修改 production 语义。T07 只负责最终稳定候选的昂贵 gate、纯机械
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
| T05C4 | [Terminal lowering cleanup](tasks/P2-T05C4-terminal-lowering-cleanup.md) | T05C6 | 高；canonical package-call lowering migration |
| T05C5 | [Terminal compile handoff repairs](tasks/P2-T05C5-terminal-compile-handoff-repairs.md) | T05C1、T05C2 | 中；input/projection 窄断链 repair |
| T05C6 | [Canonical File IR package-call target](tasks/P2-T05C6-canonical-package-call-target.md) | T05C5、用户决策 A | 高；artifact-model shared schema checkpoint |
| T05C7 | [Canonical package-call coverage](tasks/P2-T05C7-package-call-coverage.md) | T05C6 | 中；emission consumer migration |
| T05C8 | [Package call compiler consumers](tasks/P2-T05C8-package-call-compiler-consumers.md) | T05C6 | 中；driver/core consumer migration |
| T05C9 | [File IR identity version](tasks/P2-T05C9-file-ir-identity-version.md) | T05C6 | 高；identity consumer migration |
| T05C10A | [Type-only package requirement coverage](tasks/P2-T05C10A-type-only-package-requirement-coverage.md) | T05C8 | 高；driver requirement closure repair |
| T05C10B | [Package-call lowering fail-closed](tasks/P2-T05C10B-package-call-lowering-fail-closed.md) | T05C4、T05C9 | 高；lowering semantic repair |
| T05C10C | [Canonical package-call reference validator](tasks/P2-T05C10C-package-call-reference-validator.md) | T05C6 | 高；artifact-model shared validation checkpoint |
| T05C10D | [Identity package-call validation](tasks/P2-T05C10D-identity-package-call-validation.md) | T05C10C、T05C9 | 高；identity consumer |
| T05C10E | [Materialization package-call validation](tasks/P2-T05C10E-materialization-package-call-validation.md) | T05C10C、T05C7 | 高；emission consumer |
| T05C10F | [Identity checker terminal owners](tasks/P2-T05C10F-identity-checker-terminal-owners.md) | T05C9、terminal cleanup | 中；T07 checker repair |
| T05C10G | [Identity checker package-call owner coverage](tasks/P2-T05C10G-identity-checker-package-call-owner.md) | T05C10C、T05C10D、T05C10E、T05C10F | 高；checker fail-closed repair |
| T05C11 | [Orphan publication owner cleanup](tasks/P2-T05C11-orphan-publication-owner-cleanup.md) | A01 finding、T07 candidate | 高；terminal model/adapter cleanup |
| T05C12 | [Terminal compiler public-shape gate](tasks/P2-T05C12-terminal-compiler-public-shape-gate.md) | T05C11 | 高；renamed hidden-adapter negative gate |
| T03A | [Canonical contract semantic facts](tasks/P2-T03A-canonical-contract-semantic-facts.md) | terminal checkpoint、qualified alias decision | 高；typed dependency checkpoint |
| T03B | [Qualified contract type resolution](tasks/P2-T03B-qualified-contract-type-resolution.md) | T03A | 高；source type owner |
| T03C | [Contract call type checking](tasks/P2-T03C-contract-call-type-checking.md) | T03A、T03B | 高；source expression owner |
| T03D | [Terminal service-call lowering](tasks/P2-T03D-terminal-service-call-lowering.md) | T03A | 高；lowering terminal cleanup |
| T04A | [Contract-aware callable signature handoff](tasks/P2-T04A-contract-callable-signature-handoff.md) | T03B | 高；compiled/projection handoff |
| T03E | [Canonical dependency source address](tasks/P2-T03E-canonical-dependency-source-address.md) | T03A–D、`/`决策 | 高；syntax/source address checkpoint |
| T03F | [Source executable signature facts](tasks/P2-T03F-source-executable-signature-facts.md) | T03B、T03C、方案A | 高；source executable facts checkpoint |
| T03G | [File IR execution type representation](tasks/P2-T03G-file-ir-execution-carrier.md) | T03E、T03F | 高；lowering execution handoff |
| T03H | [Exact interface signature facts](tasks/P2-T03H-exact-interface-signature-facts.md) | T03F | 高；source/interface semantic checkpoint |
| T03I | [Interface File IR execution projection](tasks/P2-T03I-interface-execution-projection.md) | T03G、T03H | 高；interface execution handoff |
| T04B | [Signature handoff owner cleanup](tasks/P2-T04B-signature-handoff-owner-cleanup.md) | T03F、T04A | 中高；compiled/projection owner repair |
| T04C | [Public-instance signature owner cutover](tasks/P2-T04C-public-instance-signature-owner-cutover.md) | T03H、T04B | 高；compiled/projection public-instance checkpoint |
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
| R10 | [Canonical compiler shared fixtures](tasks/P2-R10-canonical-compiler-integration-fixtures.md) | T05C10A–G、R03、R04、R06、R11、R13 | 中；shared fixture checkpoint |
| R10A | [Shared-fixture lane contract probes](tasks/P2-R10A-shared-fixture-lane-probes.md) | R10 | 高；fan-out gate |
| R10B | [Type/import/File IR fixtures](tasks/P2-R10B-type-import-file-ir-fixtures.md) | R10A | 中；consumer batch 1 |
| R10C | [Artifact/config/DB/resource fixtures](tasks/P2-R10C-artifact-config-db-resource-fixtures.md) | R10A | 中；consumer batch 2 |
| R10D | [Contract/disposition fixtures](tasks/P2-R10D-contract-disposition-fixtures.md) | R10A | 中；consumer batch 3 |
| R10E | [Std/prelude schema fixtures](tasks/P2-R10E-std-schema-fixtures.md) | R10A | 中；从 R10B 独立出的 consumer batch 4 |
| R10G | [Shared fixture file-write owner](tasks/P2-R10G-shared-fixture-file-write.md) | R10B、R10C、R10E | 中；review abstraction repair |
| R10F | [Std package imports fixture](tasks/P2-R10F-std-package-imports-fixture.md) | R10G | 高；cargo tests blocker |
| R10H | [Typed contract fixture checkpoint](tasks/P2-R10H-typed-contract-fixture-checkpoint.md) | R10 | 中；programmatic contract input |
| R10I | [Provider/consumer contract E2E](tasks/P2-R10I-provider-consumer-contract-e2e.md) | T03E、T03I、T04C、R10H | 高；真实source验收恢复与证据刷新 |
| R12 | [Terminal compile-plane cleanup](tasks/P2-R12-terminal-compile-plane-cleanup.md) | 已吸收 | 由 clean-base reconstruction 取代 |
| R13 | [Canonical package DB schema validation](tasks/P2-R13-canonical-package-db-schema-validation.md) | T05 | 中；package DB/schema owner |
| T07 | [Phase integration gate](tasks/P2-T07-phase-integration.md) | T03A–I、T04A–C、R10H、R10I及既有terminal任务 | gate owner |
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
- T05C3 独占 T05C2 后 source orphan helper cleanup；T05C4 在 T05C6 后独占 lowering package-call consumer与
  empty index/参数链 cleanup；二者不得修改彼此或 core/facade/Cargo/checker/integration tests。
- T05C5 独占 package-only input origin/service reader 与 config requirement projection error窄修复；不得恢复
  已删除 enum/error variant，也不得修改其它 production owner。
- T05C6 独占 artifact-model canonical package-call target/external-ref schema与版本；T05C4独占lowering
  consumer，T05C7独占emission coverage consumer。三者不得互相修改或新增compatibility reader。
- T05C8 独占driver pipeline/core spawn-target直接consumer；T05C9独占artifact-identity v5 prefix/goldens；
  二者不得修改shared schema或其它consumer写域。
- R03 独占 canonical package export link 中 payload symbol 的精确投影与直接测试；map key
  继续表达 public path，link `symbol` 只能表达 file/index 指向的真实 payload declaration。
- R04 独占 canonical package config requirements 与 `ConfigShape` 的唯一 typed 表达；不为旧
  adapter/presentation 生成 source provenance、uses 或 activation 第二 owner。
- R06 独占 canonical `PackageRequirement` 闭包与 File IR package ref 覆盖校验；编译器
  内建 std 只能从同一 canonical package graph 的 std `PackageArtifact` 获得精确 version/local ABI，
  不允许 adapter 特例、硬编码 identity 或第二次 compile。
- T05C10A 独占 driver used-std/package requirement detection 与直接测试；type-only `package_symbols` 和 callable
  `package_callables` 都必须进入同一 dependency coverage，不从 callable id 猜 dependency。
- T05C10B 独占 lowering dependency-call fail-closed 与 File IR v5 lowering direct golden；不得修改 source fact
  shape 或恢复 symbol fallback。
- T05C10C 独占 artifact-model package-call reference validator 与直接测试；T05C10D/T05C10E 只消费该 API，
  不复制遍历或集合一致性规则。
- T05C10D 独占 artifact-identity validator 接入与 mutation tests；T05C10E 独占 emission/materialization 接入
  与 direct tests；T05C10F 独占 identity single-source checker 的 terminal owner graph 与 self-test。
- T05C10G 继续独占同一 checker，但只补 canonical package-call validator owner、identity/emission delegation 与
  missing/duplicate self-test；不得修改 Rust production/tests。owner existence、exclusivity 与 self-test 应从同一
  registry 派生，不能再增加一组互相漂移的手写清单。
- T05C11 独占 `compiler/compiled/**` 与 `compiler/projection-input/**` 的孤儿 publication aggregate/adapter删除，
  只保留 canonical `CompiledPackage -> ProjectionInput` handoff；不得修改 checker 或恢复 facade caller。
- T05C12 独占 compiler boundary/public-shape checker 与 self-test。必须约束 terminal public surface/field shape，
  并以 renamed aggregate/adapter 负例证明有效；不得只追加 `PackagePublication` 名字 blacklist。
- T03A独占input/source/driver的canonical contract semantic fact shape、alias namespace validation与resolved
  contract target shape；只为保持typed carrier编译可窄改直接consumer。T03B随后独占source qualified type
  resolution和exact source signature facts；T03C再独占source contract-call expression typing。T03B/T03C都
  必须拆小模块，不把新职责继续堆入数千行owner。
- T03D独占lowering旧contract operation index删除和terminal consumer；只能消费T03A target，不回读callee字符串。
  T04A独占compiled/projection-input/projection exact signature handoff与blanket Local producer删除；不回开source。
- T03E独占dependency source address AST/parser/helper和slash call consumers；type qualified path继续由T03B拥有，
  不得为旧dot dependency call留compatibility；它同时独占contract-call checker拆分与projected environment。
  T03F独占all-executable exact signature facts和public view，不修改上述call-checker owner。T03E/T03F并行时
  不得修改对方核心模块；根facade小冲突由后完成者基于checkpoint收敛。
- T03G独占source exact facts到File IR execution representation的唯一lowering投影，删除AST/display reparse和
  ServiceSymbol fallback；T04B独占compiled/projection-input/projection signature mapping/normalization cleanup。
- T03H独占source interface operation exact facts、ContractTypeId conformance与validated query API；不得修改
  lowering/compiled/projection。T03I随后独占interface lowering对T03G execution projection的复用；T04C与它
  并行，独占compiled/projection-input的source-validated public-instance handoff及canonical PackageArtifact内部
  execution target/consumer。T03I/T04C不得修改对方生产写域；T04C不得用File IR/TypeResolutionModel或legacy
  OperationAbiRef恢复semantic owner，组合证据只在共同合流后有效。
- R10 独占 `compiler/tests/common/**` shared fixture checkpoint；R10B/R10C/R10D 只能消费其 API，不能各自
  复制 compile pipeline、dependency graph、artifact reader 或 contract builder。
- R10A 在 R10 后独占 `compiler/tests/common/**` 的最后 API 修正、一个 representative lane probe target 与其
  `compiler/Cargo.toml` entry。它必须证明 type/import/File IR、config/DB/resource、explicit contract 三条 lane
  都能编译；通过后 common API 冻结，R10B/C/D 不再回开。
- R10B 独占类型/import/File IR consumer targets；R10C 独占 artifact/config/DB/resource consumer targets；
  R10D 独占 service conformance、明确删除的 targets、`compiler/Cargo.toml` 与退役 driver test-support。
  三者都不修改 production 或 `compiler/driver/service_publication_tests.rs`。只有真正验证 service protocol/
  conformance 的测试才构造显式 ServiceContract；禁止空/fake contract、provider 反推、万能聚合 builder。
- R10E 独占 `package_std_schema.rs` 与 `prelude_std_schema.rs`；它从 R10B 动态拆出，二者不得再修改对方文件。
- R10G 独占 `common/test_dir.rs` 与 review 列出的重复 file-write call sites，只做单一 IO abstraction 与机械
  consumer migration；R10F 随后独占 `std_package_imports.rs`，不得恢复 test-support/service aggregate。
- R10H在生产任务并行期间独占`compiler/tests/common/**`的typed contract dependency入口和representative probe；
  R10I只消费冻结后的common API，独占`service_conformance.rs` provider/consumer E2E，不修改production。
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
dependency call 的 dot compatibility producer/fixture = 0
旧 RemotePublicInstanceSource AST owner = 0
contract-typed executable 的 File IR ServiceSymbol/display fallback = 0
contract-typed interface operation 的 File IR ServiceSymbol/display fallback = 0
canonical PackageArtifact public-instance path 的 OperationAbiRef / File IR signature conformance owner = 0
compiled/projection-input public-instance path 的 implements_interface / package_interface_methods 重算DTO = 0
compiler production 中 PublicationAbiUnit / PackageUnit / ServiceUnit / serviceAssembly producer = 0
compiler production 中 legacy_runtime_adapter / compatibility / fallback allowlist = 0
```

## 8. 阶段验收样例

1. 无 provider source/artifact，仅凭 definition 生成、round-trip、assign/validate ServiceContract。
2. provider package只依赖 contract 即编译；显式 wrapper Available，本地 mutation/alias helper Unavailable 但
   保留 Local ABI。
3. consumer package只依赖 contract 即编译；真实调用生成一个稳定 slot、usedOperations 与 ServiceCallRef，
   artifact 不含 provider build/package/deployment/route/executable。source spelling为
   `contractAlias/operation(...)`；contract type spelling保持`contractAlias.Type`。
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
- 为让contract type进入File IR而新增Contract wire variant、ServiceSymbol/display fallback，或让runtime从opaque
  execution representation反推contract identity。
- T01/T05 后发现共享 checkpoint 缺少会改变架构语义的字段，而不是纯实现字段。

前三项直接判定方案错误；最后一项暂停受影响分支并请求用户决策。
