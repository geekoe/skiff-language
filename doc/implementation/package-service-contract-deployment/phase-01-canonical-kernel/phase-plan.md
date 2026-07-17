# Phase 01：Canonical Semantic / Identity Kernel

状态：review candidate

本阶段不引入四个目标 aggregate，也不切换 runtime 行为。它先让后续 PackageArtifact、
ServiceContract、ServiceDeployment 和 RuntimeAssembly 可以复用一套可信 leaf rules，而不是各自复制
当前 Publication 逻辑。

## 1. 阶段完成态

阶段验收时必须同时成立：

1. artifact identity 的 canonical JSON、framing、hash、assign 与 validate 有模块化单一 owner；checker
   按 crate/owner API 检查，不再要求全部实现堆在一个 `lib.rs`。
2. nominal type/interface/callable/method identity 的算法全部归 `artifact-identity`；artifact-model 只
   保存 typed input/DTO，compiler adapters 不自行 hex、拼字符串或重算 preimage。
3. `PublicationAbiUnit` 暂时保留，但 producer 和 dependency consumer 使用同一个 leaf surface validator，
   并验证 declared identity；它不得成为新目标 aggregate 的父类型。
4. Package build 与 local ABI identity 使用显式 inclusion/exclusion projection。package id/version
   coordinate 与 nominal ABI facts 进入 local ABI；recoverable、config requirement、typed effect facts 和
   完整 `fileIrIdentity` 进入 build。只排除 artifact-ref storage path、ref-level 重复 provenance 与诊断文本，
   不把 File IR owner 已纳入其 identity 的 source-map/debug content 二次过滤。
5. compiler-core 提供一个与 projection DTO 无关的 nominal type closure kernel；boundary、recoverable
   和 spawn validation 不再各自维护 walker/resolver/trace。
6. production PackageUnit 只有一个 builder/projection；package-test 不再维护第二套 ABI、export、
   identity 组装规则。
7. effect artifact leaf 能明确区分 `Unknown` 与已分析的 sound may-effects。当前未分析 callable 必须写
   `Unknown`，不能再用 empty map/placeholder 表示安全；完整 fixed-point 分析留给 Phase 02。
8. ServiceUnit/PackageUnit pointer 与当前 service assembly 的 declared hash/identity 在 compiler、runtime
   和 router load path 上使用同一 Rust owner或其 CLI；修改内容但保留 pointer 必须 fail closed。
9. 本阶段涉及的超长 identity/boundary 文件已按职责拆分；没有新增第二套 canonical JSON、ID framing、
   closure traversal、PackageUnit builder 或 effect placeholder。

## 2. 明确非目标

- 不删除 `PublicationInput`、`CompiledPublication`、`LoweredPublication`；Phase 02 负责。
- 不把 `PublicationAbiUnit` 直接改造成 `ServiceContract`；Phase 03 负责独立 contract。
- 不实现完整 callable effect fixed-point、boundary availability 或 service-call value plan；Phase 02。
- 不修改 service authoring、deployment YAML、service source compile 或 ServiceUnit code ownership；Phase 04。
- 不引入 RuntimeAssembly、ActivationContext binding vector 或 InProcessBoundary dispatch；Phase 05/06。
- 不统一所有 PackageId/ServiceId authoring UX。共享文本 grammar 不得继续扩散，distinct typed ID 在对应
  artifact 阶段落地。
- 不为旧 artifact 增加兼容 reader；fixture 直接重建。

## 3. Canonical owner 边界

```text
skiff-canonical-json (leaf utility)
  canonical key ordering / JSON number normalization / bytes

skiff-artifact-model
  typed identity inputs, artifact leaf DTOs, no hashing or string framing

skiff-artifact-identity
  identity projections, canonical preimages, hash/framing, assign/validate

skiff-compiler-core
  TypeRef traversal + nominal closure algorithm over resolver traits

compiler source/compiled
  callable semantic facts and explicit Unknown effect owner

compiler projection/emission, runtime, router
  consumers/adapters only
```

`skiff-canonical-json` 是纯 leaf crate，只负责字节规范化，不知道任何 artifact schema 或 identity prefix；
具体 identity preimage 仍只能由 `skiff-artifact-identity` 定义。

## 4. Package identity inclusion matrix

| Fact | Package local ABI identity | Package build identity |
| --- | --- | --- |
| package id / version coordinate | include | include by local ABI identity |
| public callable/type/const/instance surface | include | include by local ABI identity |
| nominal `AbiIdentityFacts` | include | include by local ABI identity |
| implementation links and File IR identities | exclude | include |
| package dependency coordinates/expected ABI | exclude | include |
| config/resource/runtime requirements | exclude | include |
| recoverable metadata | exclude | include |
| typed callable effect facts | exclude | include |
| artifact-ref storage path、重复的ref-level `sourceAstHash`、diagnostic wording | exclude | exclude |
| deployment value, service route, provider selection | exclude | exclude |

排序只由 canonical projection 负责；调用方插入顺序不能影响 identity。改变 include 字段必须改变对应
identity，改变 exclude 字段不得改变。File IR identity 自己拥有的 source-map/debug content 仍通过
`fileIrIdentity` 间接进入 package build；这里排除的是 `FileIrRef` 上重复的 provenance/storage 字段。
本阶段不定义 callable provenance DTO；Phase 02 引入 typed provenance 时，必须同步扩展 package build
projection、schema marker/prefix 与 mutation golden，且默认不进入 local ABI identity。
由于 Skiff 未发布，本阶段直接更新 prefix/golden，不保留旧算法。

## 5. Effect leaf

本阶段 artifact leaf 使用显式联合：

```text
CallableEffectSummary
  = Unknown(reasonCode)
  | Analyzed(CallableMayEffects)

CallableMayEffects
  writesCallerReachable
  returnsCallerAlias
  throwsCallerAlias
  escapesCallerValue
  requiresSameHeapIdentity
  invokesUnknownTarget
  maySuspend
```

每一项是 sound may fact，不提供“未设置即 false”的默认。Phase 01 只要求现有 compiler 为每个进入
public callable surface 的 operation 产出 `Unknown(AnalysisPending)` 并跨 compiled/projection/emission
完整保留。Phase 02 才计算 `Analyzed`；任何未来 boundary eligibility 对 `Unknown` 必须保守拒绝。
reason code 进入 build identity，诊断 detail 不进入。

## 6. DAG

```text
T01 identity module + canonical JSON foundation ──► T02 nominal/callable identity owner
                                                     │
                                                     └──► T02A identity dependency DAG contract
                                                            │
T03 typed effect leaf ──────────────────────────────► T05 package identity API/preimage
T02 ───────────────────────────────────────────────► T05

T02A + T03 + T04 + T05 ───────────────────────────► T06 PackageUnit single path + identity adoption

T06 ───────────────────────────────────────────────► T07 cross-layer reference validation

T03 + T04 + T05 + T06 + T07 ─────────────────────► T08 phase integration gate
T08 ──────────────────────────────────────────────► A01 independent acceptance
```

可并行批次：

1. T01、T03、T04 从文档 checkpoint 并行。
2. T02 从合入 T01 的 checkpoint 开始。
3. T05 从包含 T02/T03 的 checkpoint 开始，只冻结 identity API/preimage。
4. T02A 从包含 T02 的 checkpoint 开始，修正 canonical identity owner 已落地但 crate-DAG contract
   未同步的问题；它不改变 identity 算法。
5. T06 从包含 T02A/T03/T04/T05 的 checkpoint 开始，独占 compiler adoption。
6. T07 合入 T06 后执行。
7. T08 只做集成、fixture 与 gate；A01 只读验收。

## 7. 任务索引

| ID | 任务 | 依赖 | 主要 owner |
| --- | --- | --- | --- |
| T01 | [Identity module 与 canonical JSON foundation](tasks/P1-T01-identity-foundation.md) | 无 | foundation / artifact-identity |
| T02 | [Nominal/callable identity 单一 owner](tasks/P1-T02-semantic-identity.md) | T01 | artifact-model / artifact-identity / ABI builder |
| T02A | [Canonical identity dependency DAG contract](tasks/P1-T02A-identity-dag-contract.md) | T02 | compiler crate DAG / identity adapters |
| T03 | [Typed effect semantic leaf](tasks/P1-T03-typed-effect-leaf.md) | 无 | artifact-model / source / compiled |
| T04 | [Nominal type closure kernel](tasks/P1-T04-type-closure-kernel.md) | 无 | compiler-core / boundary consumers |
| T05 | [Package identity projection](tasks/P1-T05-package-identity.md) | T01, T02, T03 | artifact-identity |
| T06 | [PackageUnit 单一 projection path](tasks/P1-T06-package-unit-single-path.md) | T02A, T03, T04, T05 | compiler projection/emission |
| T07 | [跨层 artifact reference validation](tasks/P1-T07-cross-layer-validation.md) | T02, T03, T05, T06 | compiler/runtime/router/CLI |
| T08 | [阶段集成与 gate](tasks/P1-T08-phase-integration.md) | T03–T07 | integration branch |
| A01 | [独立阶段验收](tasks/P1-A01-stage-acceptance.md) | T08 | read-only acceptance |

## 8. Worktree 与冲突控制

- Integration：`/Users/geek/workspace/skiff-package-service-phase-01`，branch
  `codex/package-service-phase-01`。
- Task worktree 必须直接创建在 `/Users/geek/workspace/`，命名 `skiff-p1-tNN-*`。
- T01 独占 workspace/Cargo canonical-json 接线、artifact-identity 模块布局和 single-source checker。
- T03 独占 effect DTO、source-to-compiled handoff及现有compiler projection/emission/package-test producer的
  端到端typed wire迁移；不得改identity projection。
- T04 独占 compiler-core closure API 与 boundary/recoverable walker 迁移；不得改 artifact DTO。
- T02/T05 从新 checkpoint 顺序消费 T01；T05 独占 package identity API/preimage/prefix，不改 compiler
  projection/emission adoption callsite。
- T02A 只校正 compiler crate DAG 与 identity adapter dependency contract：允许 input/compiled 在各自边界直接
  调 canonical owner，同时删除 facade test 对 foundation crate 的旁路依赖；不得搬回或复制 identity 算法。
- T06 在 T02A 与 T05 后独占 PackageUnit builder去重与production/package-test projection收敛；消费T03
  已经迁移的effect shape，并采用T05 identity API，不得重新定义effect wire或identity算法。
- T07 才能改 runtime/router/scripts 的 identity/ref consumers，以及 compiler service dependency
  artifact loader；后者必须消费同一 canonical closure validator，不能保留独立 build-id/path fallback。
- T08 不能新增语义；语义缺口必须退回相应任务。

## 9. 验证计划

任务文件指定聚焦命令。阶段最终 commit 的唯一昂贵 gate owner 是 T08：

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-compiler-boundaries.mjs
node scripts/check-compiler-crate-dag.mjs
git diff --check
```

若 `--only runtime` 或 router 全组件测试因与本阶段无关的已知环境问题失败，T08 必须给出同 commit 的
聚焦 replacement evidence，并证明失败可在 `main` 重现；不能把新回归标为 baseline。

结构反向搜索至少包括：

```bash
rg 'SourceEffectMetadata::Empty|precision: "placeholder"' compiler artifact-model
rg 'fn canonical_json_value|fn canonical_json_number' artifact-identity compiler runtime
rg 'BoundaryPackageTypeSource|RecoverablePackageTypeSource' compiler/projection
rg 'fn build_package_unit\(' compiler
rg 'serviceAssemblyHashInput|service_assembly_hash_input' compiler runtime router scripts
```

允许命中 canonical owner、测试名和明确 legacy ledger；每个 production 命中都必须在 T08 证据中解释。

## 10. 阶段验收样例

- 同一 identity 输入的 map/vector 插入顺序不同，identity 相同。
- 改变 nominal ABI fact 会改变 package local ABI/build identity；只改 artifact path 或 diagnostic detail
  不改变。
- 相同内容使用不同 package id/version coordinate 时，local ABI/build identity 均不同。
- 改变 recoverable/effect fact（包括 normal-return 与 throw/error payload alias 的差异）只改变 build
  identity，不改变 local ABI identity。
- Publication ABI declared identity 被篡改时，dependency loader 在消费前拒绝。
- TypeRef closure 对 nullable/union/record/generic/function/any-interface 路径给出同一 trace；boundary 与
  recoverable consumer 不再拥有自定义 walker。
- 所有 public callable 至少有显式 `Unknown(AnalysisPending)`，序列化 round-trip 后不丢失。
- package-test 与 production package projection 对同一输入生成相同 PackageUnit identity。
- 修改 ServiceUnit、PackageUnit 或 service assembly 内容但保留 pointer identity，runtime/router load
  均 fail closed。

## 11. 停止条件

- 某任务需要引入第二套 identity preimage、type closure、effect inference 或 PackageUnit builder。
- 为保持测试通过必须 dual-read/dual-write 旧新 artifact schema。
- effect `Unknown` 被解释为无 effect，或诊断文本进入 identity。
- Package local ABI identity开始包含 deployment/provider/route，或 package build identity遗漏 runtime 所需
  semantic facts。
- T08 需要改变 contract/service/runtime 语义才能集成。

能由架构唯一推导的问题升级为新前置任务；会改变四对象边界的问题停止并询问用户。
