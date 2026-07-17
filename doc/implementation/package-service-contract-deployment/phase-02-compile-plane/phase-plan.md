# Phase 02：Compile Plane 实现计划

状态：review candidate

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
9. legacy service source 只做 `legacy input -> PackageCompileInput -> PackageArtifact -> legacy runtime adapter`；
   不二次解析、type/effect analysis 或 lowering。Phase 01 `PackageUnit` 只允许作为该 adapter 的 runtime
   外壳和 runtime 现有 typed DTO，不再是 canonical compiler output。
10. 本阶段直接触碰的 publication compile owner、重复 identity/projection、超长新增逻辑均已删除或拆分；
    结构 gate 阻止 legacy adapter 扩散。

## 2. 本阶段实现选择

架构明确不冻结 contract authoring 文件和 CLI。本阶段选择最小 typed 边界，避免提前发明用户语法：

- 新增严格 typed `ServiceContractDefinition` Rust 输入，可由测试、未来 CLI 或平台 producer 构造。
- Phase 02 不新增 `contract.yml`、IDL 或源码 declaration 语法；Phase 05 再为 typed API 接 authoring UX。
- `ContractTypeId` 是独立 tagged identity，不复用或改名 `AbiTypeId`。artifact type ref 以显式 contract
  symbol/reference 表达，schema closure 通过 ContractTypeId 查找，不靠 display string 猜测。
- contract operation identity 由 service coordinate、contract version 和稳定 operation key 派生；
  ServiceProtocolIdentity 由完整 operation descriptor 与 closed schema canonical projection 派生。
- provider/build/deployment/route、`BoundaryImplementationRequirements` 和诊断文本不进入 protocol identity。
- `BoundaryOperationDescriptor` 的 error/stream/cancel/callback/value-plan/effect guarantee 字段全部显式；当前
  语言暂不支持的 lane 使用 tagged unavailable/unsupported 状态，不能通过字段缺失表达。
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
  only PackageCompileInput pipeline + explicitly named legacy service adapter
```

旧 `compiler/projection/src/contract/**` 中纯 schema/type-closure 算法可以抽出复用；含 implementation binding、
service config 或 runtime projection 的 aggregate 不得成为 ServiceContract owner。

## 4. DAG 与三个实现波次

```text
T01 canonical compile-contract checkpoint
  ├── T02 sound effect/provenance analysis
  ├── T03 contract requirement + ServiceCallRef lowering
  └── T04 PackageArtifact + boundary projection

T02 + T03 + T04
  ├── T05 package-only compiler cutover
  └── T06 legacy runtime/test consumer adapter

T05 + T06 -> T07 phase integration gate -> A01 independent acceptance
```

| 波次 | 并行任务 | 说明 |
| --- | --- | --- |
| 1 | T01 | 独占 artifact-model、artifact-identity、typed contract leaf 与公共 schema/API |
| 2 | T02、T03、T04 | 分别独占 source effect、input/lowering、projection/emission；不修改中央 driver |
| 3 | T05、T06 | T05 独占 compiler hot roots；T06 独占 legacy adapter/test-runner/runtime test consumers |

T07 只负责最终稳定候选的昂贵 gate、机械 fixture 和结果记录，不新增语义。A01 只读验收。

## 5. 任务索引

| ID | 任务 | 依赖 | 风险 / 验收组 |
| --- | --- | --- | --- |
| T01 | [Canonical compile-contract checkpoint](tasks/P2-T01-canonical-compile-contract.md) | 无 | 高；独立 checkpoint |
| T02 | [Sound effect/provenance analysis](tasks/P2-T02-effect-provenance.md) | T01 | 高；effect group |
| T03 | [Contract requirement 与 ServiceCallRef lowering](tasks/P2-T03-service-call-lowering.md) | T01 | 高；dependency group |
| T04 | [PackageArtifact 与 boundary projection](tasks/P2-T04-package-artifact.md) | T01 | 高；artifact group |
| T05 | [Package-only compiler cutover](tasks/P2-T05-compiler-cutover.md) | T02–T04 | 中；compile cutover batch |
| T06 | [Legacy runtime/test consumer adapter](tasks/P2-T06-legacy-consumers.md) | T02–T04 | 中；consumer batch |
| T07 | [Phase integration gate](tasks/P2-T07-phase-integration.md) | T05–T06 | gate owner |
| A01 | [Independent stage acceptance](tasks/P2-A01-stage-acceptance.md) | T07 | 独立只读验收 |

## 6. 写入冲突规则

- T01 独占 `artifact-model`、`artifact-identity`、新 contract leaf crate、root workspace/API policy。波次 2
  不修改这些公共 wire；发现缺字段必须回报 T01 checkpoint amendment。
- T02 独占 `compiler/source` effect/provenance 新模块及直接 source tests；不修改 lowering、projection、emission、
  driver。中央 source facade 的最小 export 由 T02 提交，T05 后续可机械改名但不改算法。
- T03 独占 contract dependency reader、dependency operation index、service-call lowering/external refs；不修改
  source effect、package projection或driver。
- T04 独占新的 PackageArtifact projection/materialization、boundary projection 与直接 emission tests；不修改
  source/lowering/driver。
- T05 独占 `compiler/driver/pipeline`、input-model compile input、compiled/lowering/source 根类型、projection
  facade、compiler facade 和 compiler structure checker。只消费 T02–T04 API。
- T06 独占明确命名的 legacy runtime adapter、test-runner、package-test consumer 和相关 fixtures；不修改
  canonical identity/effect/lowering。
- 超过数百行的新 owner 要按 model/validation/projection/tests 拆文件；同一规则在两个模块出现即暂停并抽
  canonical leaf，不能以时间为由复制。

## 7. 验证计划

任务只运行自己的聚焦命令。最终稳定候选的唯一昂贵 gate owner 是 T07：

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only test-runner
node scripts/verify.mjs --only runtime
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

结构 gate 至少证明 production canonical path 中：

```text
PublicationInput / PublicationKind / CompiledPublication / LoweredPublication = 0
PackageArtifact 或 ServiceContract 内 PublicationAbiUnit / ServiceUnit = 0
contract-only consumer path 的 provider build/deployment/route/executable target = 0
canonical File IR 的旧 ServiceDependencySymbol producer = 0
legacy DTO import 只存在明确 adapter/runtime allowlist
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
7. legacy service fixture只做一次 source compile，adapter 与 canonical PackageArtifact 共享 File IR/build facts。
8. package-test/test-runner 精确复用 production PackageArtifact identity，不维护第二套 builder。

## 9. 明确非目标

- 不选择 provider、不生成 ServiceDeployment 或 RuntimeAssembly。
- 不执行 service call，不实现 ActivationContext 或 InProcessBoundary runtime。
- 不定义最终 contract YAML/IDL/CLI authoring UX。
- 不物理删除 runtime 仍消费的全部 PackageUnit/ServiceUnit DTO；只收缩为明确 adapter/runtime input，Phase 03
  删除 code-owning ServiceUnit，Phase 05 删除剩余 adapter。
- 不追求完美 effect precision；只要求 sound、可证明简单路径可用、未知路径 fail closed。

## 10. 停止条件

- 需要把 provider package/build/deployment/route 写进 ContractRequirement、ServiceRequirement 或 ServiceCallRef。
- 为兼容旧 runtime 必须让旧 DTO重新拥有 identity/effect/projection规则或双写 canonical artifact。
- effect 分析以无法证明的 false 表示安全，或 Unknown 被解释为无 effect。
- PackageArtifact/ServiceContract 通过嵌入 PublicationAbiUnit/ServiceUnit 快速完成。
- T05/T06 发现共享 checkpoint 缺少会改变架构语义的字段，而不是纯实现字段。

前三项直接判定方案错误；最后一项暂停受影响分支并请求用户决策。
