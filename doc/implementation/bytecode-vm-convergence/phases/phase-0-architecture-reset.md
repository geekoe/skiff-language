# Phase 0：架构重置与垂直验证基础

> Status: integrated; acceptance blocked
>
> Depends on: project plan accepted for execution
>
> Unblocks: Phase 1 design and implementation only after the
> [supplemental closure task](../tasks/phase-0-supplemental-closure.md) passes

Phase 0 不修复全部 VM 问题，也不扩展 bytecode 支持面。它负责把第一次实施中缺失的前置工作补齐：
确认真实 baseline、关闭顶层架构决定、分类当前可达能力、找到或建立 Phase 1 的垂直闭环证明，并把
Phase 1 细化到可以安全派发 Agent 的程度。

本 Phase 采用[大型重构实施原则](../large-change-execution-principles.md)和[项目总计划](../README.md)。

## 1. 目标

Phase 0 完成时必须同时具备：

1. 当前 compiler -> artifact -> runtime -> response 真实路径图；
2. semantic verifier / seal 的明确 disposition；
3. Phase 1 目标 pipeline 和唯一 authority map；
4. 当前 capability ledger 与 containment 决策；
5. Phase 1 最小支持面和所有 unsupported lane 的 fail-closed owner；
6. Phase 1 Test Design Specification 和 semantic coverage matrix；
7. 可执行的 Phase 1 VCP harness 与 canonical Phase Gate command，或对其缺失的明确 blocker 决议；
8. 从第一次派发起持续维护的 Phase 0 Execution Map 和 revision record；
9. Phase 1 Semantic Closure、task DAG、write-set/角色分离/worktree 约束和 `MAP1` 建立条件；
10. 独立设计审查 PASS；
11. exact Phase 0 result 和 Phase 1 handoff。

## 2. 非目标

Phase 0 不：

- 修复 aggregate COW、exception、scheduler、HTTP、stream、task、Actor 或 GC；
- 为现有 verifier 增加 proof phase；
- 增加 opcode、generic facade 或兼容 artifact 路径；
- 恢复 tree evaluator 或建立双执行器；
- 宣称 production VM 已正确；
- 运行 destructive retirement；
- 在没有验证计划的情况下提前派发 Phase 1 production implementation。

若建立 VCP 需要少量代码，Phase 0 只允许 validation infrastructure、deterministic fixture 和只读
observability。它不能增加 test-only execution path、手写 executable image、fake verifier seal 或绕过
production loader/linker/VM 的入口。

## 3. Phase 0 必须关闭的决定

### D0-01 — Semantic verifier disposition

工作方向是删除当前 broad semantic verifier、`VerificationSeal` 和它作为 execution authority 的角色，保留：

- 不可信 artifact 的 bounded structural decode/validation；
- exact deployment/package/registry linking；
- VM 执行所需的 runtime invariant checks。

Phase 0 必须比较并选择：

1. **删除 semantic verifier**：linker 产出 immutable executable image，VM 不依赖 seal；或
2. **保留薄 verifier**：仅在能够列出不可由 structural validator、linker 或 VM owner 替代的具体职责时。

无论选择哪项，都禁止 verifier：

- 从字符串或 opcode 重建 native/interface/callback authority；
- 为 linker normalization failure 提供 type equivalence；
- 从静态类型猜 actual exception identity；
- 从类型外形生成 lifecycle plan；
- 为 unsupported lane 制造可执行 seal。

产出必须包括保留/删除的 crate、public type、cache key、admission path 和迁移顺序。若 Phase 0 无法唯一
决定，状态为 `blocked`，向用户提出最小决策问题。

### D0-02 — Executable image 和 admission chain

冻结 Phase 1 的唯一链路及每一层输入/输出：

```text
source facts
  -> relocatable artifact
  -> structurally admitted view
  -> exact linked executable image
  -> exact entry pin
  -> VM
  -> request response
```

必须说明 deployment build ID、package build ID、entry identity、artifact bytes 和 image cache 的唯一 owner，
并删除“执行时重新读取 artifact root”或“用 package build 代替 deployment build”的计划空间。

### D0-03 — Phase 1 MVP surface

Phase 0 根据真实代码确定 Phase 1 最小集合。默认候选是：

- scalar literal、slot、算术/比较和 branch；
- exact non-generic local call；
- unary operation/gateway entry；
- return；
- hard raw fuel 和 basic request deadline/internal-stop poll；
- 不需要 Pending 的 deterministic response projection。

aggregate mutation、ordinary throw/catch、host effect、stream、task、service/Actor/interface/callback、InOut、
generic specialization、request GC 默认不进入 Phase 1。若 scalar VM 当前依赖其中某项，Phase 0 要么把最小
依赖纳入完整 closure，要么重新选择更小 VCP；不能把依赖藏在 test fixture 中。

### D0-04 — Capability containment

为 architecture review 的每项问题和每个 production ingress 建立 ledger：

| Capability | Current reachability | Current state | Phase owner | Phase 0 action |
| --- | --- | --- | --- | --- |
| scalar/local execution | audit | audit | Phase 1 | retain candidate path |
| aggregate/lifecycle | audit | expected enabled-unaccepted | Phase 2 | define fail-closed/containment |
| throw/catch/unwind | audit | expected enabled-unaccepted | Phase 3 | define fail-closed/containment |
| Pending/session ownership | audit | mixed | Phase 4 | prevent new use |
| HTTP/resource/stream | audit | expected enabled-unaccepted | Phase 5 | define containment |
| task/service/interface/callback/Actor | audit | mixed/unsupported | Phase 6 | one gate per lane |
| GC/performance | audit | planned/latent | Phase 7 or split | no premature enablement |

Phase 0 的目标是决定 containment 边界，不要求在本 Phase 完成所有 production code 修改。若某条当前可达
路径会造成数据破坏、无限阻塞或无法取消，Phase 0 必须把 containment 提升为 Phase 1 前置 kernel task，
不能记为普通 follow-up。

### D0-05 — Phase 1 test architecture、VCP 和 Gate

必须关闭：

- Test Design Specification 和 coverage matrix owner；
- VCP 入口、终点和 scenario；
- fixture、failure injection 和只读 observability；
- evidence manifest schema；
- canonical selector / script；
- Phase Gate 聚合哪些 evidence class；
- gate command owner 和 independent acceptance owner。

详见 §6。

## 4. 调查任务

所有调查从同一个 exact baseline commit/tree 开始，只读执行，不需要 worktree。调查 Agent 不继承其他
调查 Agent 的推断，只提交证据和未决问题。

| ID | Owner type | Scope | Required output |
| --- | --- | --- | --- |
| AUD0 | baseline owner | repo identity、dirty state、binary/artifact/test topology | exact baseline receipt |
| AUD1 | pipeline investigator | compiler handoff、artifact、loader、linker、verifier/image/cache | actual producer-to-entry graph and duplicate authority list |
| AUD2 | VM investigator | admission、dispatch、fuel、slot/local call/return、runtime invariant | minimal executable core and hidden dependencies |
| AUD3 | runtime investigator | request entry、scheduler、host、boundary response、session ownership | actual request-to-response graph and blocking/bypass points |
| AUD4 | validation investigator | test-runner、skiff-tests、HTTP fixture、isolated runtime harness、verify selector graph | VCP/gate candidate inventory and gap analysis |
| AUD5 | containment investigator | all production ingress vs architecture findings | capability ledger and urgent containment list |

调查可并行。每个输出必须包含文件/符号/commit 证据、可触发性、masking relationship，不能只给架构建议。

## 5. 设计与任务 DAG

```text
MAP0 v0: baseline + ready audit frontier
  -> {AUD0 baseline, AUD1 pipeline, AUD2 VM, AUD3 runtime,
      AUD4 validation, AUD5 containment} parallel audit join
  -> DEC0 architecture decision packet
  -> TST0 Phase 1 Test Design Specification
  -> REV0-D independent design review
  -> MAP0 refresh for reviewed ready frontier
  -> HAR0 executable proof harness / gate
  -> MAP0 refresh before next dispatch
  -> PLN1 Phase 1 detailed plan
  -> REV0-F readiness review
  -> RES0 Phase 0 result
```

### MAP0 rolling Execution Map

`MAP0` 不是派发给某个具名角色的实现任务，而是执行本 Phase 时的前置和持续义务。派发第一个调查任务前，
必须创建 `tasks/phase-0-execution-map.md`；之后每次派发新的 ready frontier 前先更新它。本文只提供约束，
不预先指定实际 Agent、branch、worktree 数量、路径或派发顺序。

MAP0 初始版本只需要闭合 baseline 和 AUD0–AUD5 的当前派发批次。后续版本至少记录：

- exact baseline/current integration commit；
- ready、blocked、conditional 和已完成 task；
- 每个已派发 task 的实际 Agent、输入 commit、write/read set、branch 和 worktree；
- 合流顺序、交付 commit、验证责任和下一次可派发条件；
- 每次调整的原因和受影响节点。

Agent 更换、串并行调整、worktree 增减、leaf 拆并或合流顺序变化直接更新 MAP0。若新情况要求改变
Phase 1 support surface、Semantic Closure、authority、中央接口、TST0、VCP 或 Gate，则停止派发，回到 DEC0
或 TST0 并重新执行受影响的独立审查。冻结 candidate 后再调整 MAP0 会解冻 candidate，并开始新的 evidence
epoch。

### DEC0 architecture decision packet

一个 design owner 统一消费所有审计，提交：

- verifier disposition；
- target pipeline；
- exact authority table；
- Phase 1 support/disabled matrix；
- containment blockers；
- Phase 1 test architecture、VCP specification 和 Gate contract。

DEC0 不直接实现 production code。

### TST0 Phase 1 Test Design Specification

一个 test design owner 把 DEC0 的支持面和 invariant 转成可审查的 semantic coverage matrix。至少覆盖：

| Dimension | Required Phase 1 scenarios |
| --- | --- |
| source/emission | literal、slot、branch、exact local call、return；unsupported construct 拒绝 |
| artifact/admission | canonical artifact；size/index/target corruption |
| exact linking/image | exact deployment/entry；missing/mismatch；无 first-package/first-operation fallback |
| VM execution | operand/slot/local frame/result；fuel exhausted；deadline/internal stop poll |
| request boundary | deterministic success；VM failure projection；terminal exactly once |
| lifecycle hygiene | request cleanup；无 Pending/resource/child owner 泄漏 |
| structure | 无 semantic seal bypass、type equivalence、ambient artifact reread 或 alternate executor |

每个 cell 必须映射到 fixture、test level、observable assertion、failure injection 和 owner。对于 Phase 1 明确
不支持的维度，matrix 证明它在唯一 capability gate fail closed，而不是简单写“未测试”。

TST0 还必须给出现有相关测试的 disposition：保留为 canonical scenario、降为 focused helper、合并重复
fixture、或因旧模型失效而删除。不能一边新增 VCP，一边保留多套不同 artifact/image/request 模型的测试
harness。

TST0 必须在 HAR0 和 Phase 1 production task 之前通过独立设计审查。

### HAR0 executable proof harness and gate

若 AUD4 找到现有合格 harness，HAR0 只补必要 observability、fixture registration 和 evidence manifest。若没有，
先给出两个最小方案及 trade-off，由 design review 判断是否仍属 validation infrastructure：

1. **in-process production composition harness**：使用 production compiler/store/loader/linker/request entry，
   只把 clock/store/host completion 换成 deterministic implementation；
2. **isolated runtime harness**：构建真实 compiler/runtime binary，以动态端口和临时 artifact root 运行一份
   unary fixture。

若两者都要求新增 production execution API 或改变 runtime ownership，本 Phase 标记 `blocked`，先与用户讨论，
不能把它藏进 Phase 1 leaf。

HAR0 的可执行交付至少包括：

1. 真实 `.skiff` fixture/corpus；
2. production-shaped harness；
3. VCP evidence manifest 及 schema checker；
4. deterministic negative mutation/failure injection；
5. 注册在 `scripts/verify.mjs` selector graph 或等价 canonical registry 中的唯一入口；
6. Phase Gate wrapper/checker，聚合 TST0 指定的 evidence，拒绝 skip、零场景和 stale epoch。

测试可以分布在 compiler/runtime/test-runner/skiff-tests，gate command 必须唯一。验收者不应手工执行多条
命令后自行拼接结论。

### PLN1 Phase 1 detailed plan

只有 DEC0 和 HAR0 closure 后才编写。PLN1 必须包含：

- Phase 1 Semantic Closure；
- central kernel 和 leaf DAG；
- exact interfaces，不接受 placeholder；
- write-set 边界、必须保持的角色分离和 worktree 隔离约束；
- Phase 1 启动时建立 `MAP1` 的条件和首个 ready frontier；实际 Agent、branch 和 worktree 留给 `MAP1`；
- TST0 coverage matrix，以及 VCP、focused、negative、structural 和 regression gates；
- frozen candidate 和 acceptance independence 要求；
- Phase 2 输入和 Phase 1 不得触及的文件/能力。

### REV0 independent reviews

由未参与 DEC0/HAR0 production write 的 Agent 分两次只读审查：

- `REV0-D`：在 HAR0 前审查 architecture decisions、Test Design Specification 和 VCP/Gate contract；
- `REV0-F`：在 HAR0/PLN1 后审查可执行验证方式和 Phase 1 implementation readiness。

两次审查合计至少检查：

- architecture review 的关键问题是否被 target topology 正面解决；
- Phase 1 是否真的可以不依赖未来 lane；
- verifier disposition 是否减少而不是转移多重 authority；
- VCP 是否经过真实 composition；
- TST0 是否覆盖 Phase 1 声明的全部 semantic dimension 和 state transition；
- HAR0 是否真的落成 canonical executable selector，而不是文档命令清单；
- task DAG 是否以 Semantic Closure 而非 crate 列表为核心；
- containment 是否覆盖当前危险的 enabled-unaccepted path；
- MAP0 是否只细化 ready frontier，且 worktree 和 acceptance independence 约束可执行。

任一 blocker 使 Phase 0 `blocked`；修复后产生新的 design evidence epoch，并从受影响的 review checkpoint
重新审查。

## 6. Phase 1 垂直闭环证明规范

### 6.0 与门禁的关系

VCP-1 是 Phase 1 Gate 的必要子证明。Phase 1 Gate 的逻辑是：

```text
exact frozen candidate
  + focused/unit evidence
  + producer-consumer contract evidence
  + VCP-1 manifest
  + negative/lifecycle matrix
  + structural no-bypass checks
  + required regression selectors
  = PASS | FAIL
```

Gate checker 必须机械校验 candidate identity、scenario count、skip count、manifest schema 和每个子证明状态。
VCP-1 success 不能覆盖其它子证明失败。

### 6.1 名称

本项目使用 **VCP-1：Trusted Scalar Execution Closure**。它不是最终 whole-system E2E，而是 Phase 1
完整责任链的 production-shaped proof。

### 6.2 最低路径

```text
real .skiff source fixture
  -> production compiler pipeline
  -> immutable artifact store record
  -> production structural admission
  -> production exact linker / image construction
  -> production request entry and exact entry selection
  -> VM scalar/local execution
  -> production response projection
  -> deterministic externally asserted payload
```

不允许：

- 直接构造 `LinkedBytecodeCandidate`、verified/executable image 或 VM fiber；
- test-only seal、unchecked entry constructor 或 fake deployment owner；
- compiler 与 runtime 共享内存中的未持久化内部 DTO；
- 绕过 production request entry 直接调用 dispatch loop 作为唯一证据；
- 最终只断言 HTTP 200，不证明 exact build/entry/VM path。

### 6.3 必须观测的事实

VCP evidence manifest 至少记录：

- source fixture identity；
- compiler/runtime binary identity；
- artifact schema 和 content/build identity；
- exact deployment build ID 和 selected entry；
- image admission/link outcome；
- VM dispatch count 或等价不可伪造 marker；
- response payload；
- fallback/bypass count 为不存在，而不是仅为零；
- request terminal 和资源清理结果。

测试专用只读 event sink 可以承载这些事实；event sink 不得选择执行路径或制造 owner。

### 6.4 Negative companion

同一个 harness 必须至少证明：

1. 损坏 artifact/index/target 在 production admission 边界失败；
2. Phase 1 之外的一个 opcode/capability 在唯一 capability gate fail closed；
3. exact deployment/entry mismatch 不会选择另一个 package、首个 operation 或 ambient artifact。

### 6.5 找不到 VCP 时的停止条件

AUD4/HAR0 若不能满足 §6.2–6.4，Phase 0 不得 `accepted`。报告必须明确问题属于：

- 缺 compiler-to-store handoff；
- 缺 production composition API；
- 缺 deterministic process harness；
- 缺只读 observability；
- 当前 Phase 1 scope 仍过大；
- 必须先修复的 production ownership blocker。

然后向用户提交最小方案选择。不能用更多 unit test 或 verifier proof 替代。

### 6.6 Phase 0 red baseline 和 Phase 1 green gate

Phase 1 开始前要求的是**可执行、可信的验证方式已经存在**，不要求尚未修复的 Phase 1 行为伪装成绿。

- Phase 0 必须运行 HAR0 command，证明 harness 能穿过预期 composition，或在已知 Phase 1 blocker 上产生可定位
  的 expected-red evidence；
- expected-red 必须由一个已知 control case、negative injection 或现有 accepted path 证明 harness 本身有效；
- Phase 0 result 明确记录 red 的责任边界，不能把它计为 PASS；
- Phase 1 frozen candidate 必须让完整 Gate（含 VCP-1）变绿，才能 accepted。

若 harness 连目标边界都到不了、无法区分测试设施失败和产品失败，Phase 0 仍是 `blocked`。

## 7. Execution Map 和 worktree 约束

### 7.1 Phase 0

- 第一次派发前建立 MAP0；任何未出现在当前 MAP0 ready frontier 的任务不得启动；
- main checkout 始终留在 `main`；Phase 0 使用唯一 integration line，实际 branch/path 由 MAP0 记录；
- AUD0–AUD5 只读，不要求每个 Agent 各建 worktree；若 main 不能保持 exact baseline，MAP0 分配一个共享的
  detached baseline worktree；
- 并发 write owner 不得共享 worktree；串行且 write set 不冲突的任务可以在 MAP0 明确交接同一 worktree；
- HAR0 只有在需要写 validation infrastructure 时才需要独立 leaf worktree；
- REV0-D/REV0-F 在 MAP0 记录的 exact commit 上只读，不修改受审 candidate；
- frozen gate 使用 detached、只读 worktree，且最终 acceptance 不得由本 Phase production 实现者完成；
- 所有 worktree 直接建立在 `/Users/geek/workspace` 下，具体名称和数量由 MAP0 决定。

### 7.2 Phase 1 handoff

PLN1 不预先分配 Phase 1 的实际 Agent 或 worktree。Phase 1 第一步是依据 accepted PLN1 建立 `MAP1`，当时再
根据 exact baseline、ready frontier、可用 Agent 和写冲突决定具体拓扑。Phase 1 仍必须有唯一 integration
line 和 frozen gate worktree；实现者不能担任最终 acceptance owner。

## 8. 验收

Phase 0 只有在以下条件全部成立时才能 `accepted`：

- [ ] exact baseline receipt 已记录；
- [ ] MAP0 在第一次派发前建立，且 revision history 覆盖所有派发、调整和合流；
- [ ] D0-01 至 D0-05 全部关闭；
- [ ] capability ledger 覆盖所有 production ingress 和 review findings；
- [ ] target pipeline 每个 fact 只有一个 owner；
- [ ] TST0 coverage matrix 覆盖 Phase 1 全部声明支持/拒绝维度；
- [ ] VCP-1 harness 和唯一 canonical gate command 已落地；
- [ ] HAR0 command 已执行并产生可信 green 或明确 expected-red baseline；
- [ ] VCP-1 negative companion 能在预期边界失败；
- [ ] Gate checker 拒绝 skip、零场景、缺 manifest 和 stale candidate evidence；
- [ ] 没有 test-only execution bypass；
- [ ] urgent containment 已成为 Phase 1 前置 task 或已完成；
- [ ] Phase 1 detailed plan 含 task DAG、write-set/角色分离/worktree 约束、`MAP1` 建立条件和 gates；
- [ ] REV0-D design review 和 REV0-F readiness review 均 PASS；
- [ ] Phase 0 result 记录 exact candidate commit/tree 和 evidence epoch。

若当前代码无法让 VCP success case 通过，但 harness 已证明失败发生在一个 Phase 1 应修复的明确边界，
Phase 0 只能把 **validation readiness** 标为通过，不能把 VCP behavior 标绿。必须决定：缩小 Phase 1
success surface、在 Phase 0 修复纯 validation seam，或将该 blocker 明确纳入 Phase 1 kernel。该选择需要
在 result 中单独说明，不能用“后面再集成”带过。

## 9. Result 最低内容

Phase 0 result 应创建在本项目后续 `results/phase-0.md`，至少包含：

- baseline commit/tree 与 repo state；
- MAP0 path、最终 revision 和重要调度变化；
- audit task evidence links；
- DEC0 decision record；
- capability ledger；
- TST0 coverage matrix；
- VCP-1 canonical gate command、fixture、manifest 和 negative evidence；
- conditional HAR0 commits；
- REV0-D/REV0-F reviewer verdict；
- Phase 1 exact plan path 和 implementation-ready verdict；
- open blocker 和用户决定（如有）。

Phase 0 result 不得宣称任何 VM production capability 因文档设计本身成为 `accepted`。
