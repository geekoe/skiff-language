# Phase 1：最小可信同步执行闭环

> Status: activation authorized by the accepted Phase 0 closure; MAP1 v0 required before production dispatch
>
> Semantic Closure: Trusted Synchronous Execution Closure
>
> Depends on: [`phase-0-closure.md`](../results/phase-0-closure.md) accepted
>
> Unblocks: Phase 2 value lifecycle and writable path

本文在“Phase 0 已有效 accepted”的假设下定义 Phase 1。实际执行时不能仅相信状态文字；主 Agent 必须读取
Phase 0 closure result、acceptance receipt、exact candidate/tree、shared Phase Contract 和 target handoff。任一
输入缺失、互相矛盾或不属于同一 evidence epoch，Phase 1 保持 `blocked`。

Phase 1 只建立一个很小但真实的同步 production execution closure。它不追求 opcode 数量，而是让 source、
artifact、loader/linker、immutable image、VM、budget 和 request boundary 在同一支持面内只有一个 authority、
一个执行入口和一个可重复证明。

本 Phase 采用[大型重构实施原则](../large-change-execution-principles.md)、[项目总计划](../README.md)及 Phase 0
accepted Contract/receipt。若本文与 Phase 0 exact receipt 冲突，以 receipt 为准，并在启动实现前修订本文；
不能由 leaf Agent 自行选择。

## 1. 目标

Phase 1 完成时必须同时具备：

1. 当前危险的非 Phase 1 capability 已在 production 唯一边界 containment；
2. source scalar/local facts 经 compiler 和 artifact 无损传到 exact linker/image/VM；
3. broad semantic verifier/seal 不再是 execution authority；保留的薄 proof stage 只能执行 Phase 0 handoff 明列职责；
4. release/deployment/package/entry identity 沿 production route exact pin，不使用 ambient root 或 first-match；
5. VM 只从 immutable executable image 的 pinned entry 进入；
6. scalar slots、branch、exact non-generic local call、frame return 和 deterministic unary response 正确；
7. hard raw fuel、deadline 和 internal-stop 在受信 dispatch boundary 不可绕过；
8. supported synchronous path 不产生 Pending、resource、child owner 或第二执行器；
9. VCP-1 经过 Phase 0 accepted production composition seam，并覆盖真实 local call；
10. canonical Phase 1 Gate 聚合 focused、contract、VCP、negative、structural、budget 和 regression evidence；
11. frozen candidate 由全新 Acceptance Agent 独立验收；
12. 所有 Phase 1 以外 capability 保持 `disabled`，不会因已有 match arm 被误记为 accepted。

## 2. 非目标

Phase 1 不：

- 实现 aggregate snapshot/COW、container lifecycle、writable path 或 ordinary aggregate drop；
- 开启 string/bytes/record/Array/Map，除非 Phase 0 accepted support matrix 明确将某个形态证明为 immediate scalar；
- 实现 ordinary throw/catch、region unwind 或 recoverable error envelope；
- 接入 host effect、HTTP、stream、真实 Pending 或 scheduler child；
- 开启 task、service、Actor、interface、callback、`InOut` 或 generic specialization；
- 实现 request GC、cross-owner heap、resource lifetime 或 unified memory ledger；
- 完成所有 release benchmark 或优化全部 VM hot path；
- 保留 verifier、type equivalence、字符串 dispatch 或 test-only constructor 作为临时兼容；
- 因某能力“代码已经存在”而把它加入支持面。

发现最小 scalar closure 依赖上述能力时，必须缩小 fixture、补充 earlier containment，或按 §14 停止；不能
把依赖隐藏在 compiler builtin、default package、test fixture 或手写 image 中。

## 3. Phase 0 输入和启动检查

### 3.1 Required inputs

MAP1 v0 必须引用同一 accepted Phase 0 closure epoch 的：

- `results/phase-0-closure.md`；
- exact Phase 0 candidate commit/tree 和 post-acceptance merge commit；
- Phase 0 shared Phase Contract 和独立 Acceptance receipt；
- verifier/image/entry、authority、containment 的唯一 accepted handoff；
- accepted production composition seam；
- durable Phase 0 Gate manifest/raw evidence hashes；
- current capability ledger；
- Development/Proof 两条线的 first ready task handoff。

### 3.2 Activation preflight

派发 production task 前必须证明：

- main checkout 在 `main` 且 clean；
- Phase 1 baseline 是 Phase 0 accepted merge 的 descendant；
- Phase 0 后没有改变 compiler/artifact/linker/image/VM/request/Gate contract 的未审查 commit；
- current production route 与 Phase 0 receipt 仍一致；
- Phase 1 support surface 和 disabled surface 没有 unresolved item；
- 首批 Development/Proof Agent、worktree 和 write set 已进入 MAP1；
- K0 containment 是第一个 production frontier；
- Acceptance Agent 尚未参加任何 Phase 1 candidate 写入。

Phase 0 receipt/identity 无效时整体 blocked。一个当前事实不清楚时按 §8 启动 Clarification；一个共享目标选择
未决定时只阻塞其消费者并条件启动 Design。不能因为局部问题停止所有无关 ready task。

## 4. 精确支持面

Phase 1 最低支持集合如下；Phase 0 accepted matrix 可以进一步缩小，但扩大必须修改共享 Phase Contract，并在
触发 §8.2 高风险条件时经过独立 review。

| Dimension | Accepted target |
| --- | --- |
| value shapes | `number`、`boolean`、`null` 等 Phase 0 明确证明为 immediate scalar 的 closed set |
| source flow | scalar literal、local scalar binding/slot、numeric arithmetic/comparison、boolean branch、return |
| calls | exact non-generic direct local call；同一 VM dispatch loop、非 Rust recursion |
| entry | exact unary operation/gateway entry；exact deployment/operation pin |
| result | deterministic scalar JSON payload或 Phase 0 明定的 canonical scalar boundary carrier |
| artifact | bounded structural admission；只消费 exact schema/registry pins |
| executable image | exact deployment closure、immutable、entry pinned、无 ambient reread |
| control | finite raw fuel、deadline/internal-stop poll、single terminal |
| physical state | request-local synchronous frame/value state；无 Pending/resource/child owner |

默认不支持并必须 fail closed：

- aggregate/string/bytes/collection/record values；
- `tail_call_local`，除非 Phase 0 accepted matrix 已给出 exact eligibility 和 bounded diagnostic proof；
- throw/rethrow/exception region；
- host effect、stream、callback/interface/service/Actor/task；
- generic、`InOut`、recoverable、request GC；
- 任何会返回 `VmControl::Pending`、`EnterChild` 或 provider resource 的 opcode/target。

Capability gate 必须按 exact executable entry closure 判断，不能只检查 request mode，也不能因为 unsupported
function 当前不可达就把未知 opcode/target 留进 accepted image。

## 5. Authority 和目标拓扑

Phase 1 的唯一逻辑链是：

```text
source-owned scalar/local facts
  -> compiler lowering/emission
  -> relocatable artifact
  -> bounded structural admission
  -> exact deployment linker
  -> Phase-0-defined immutable executable image
  -> exact deployment + operation entry pin
  -> synchronous VM core
  -> production request response
```

| Fact / behavior | Sole owner |
| --- | --- |
| scalar type、direct-local target、effect、source site | source analysis/lowering |
| opcode/operand/schema/index/resource limits | artifact model + structural validator |
| deployment/package/registry/relocation resolution | exact deployment linker |
| executable image identity、cache、entry pin | deployment image owner defined by Phase 0 handoff |
| frame、slot、PC、local call、return、dispatch | VM core |
| raw fuel、deadline/internal-stop poll | trusted VM budget boundary + request policy |
| unary route、terminal response、request cleanup | request/host production boundary |
| VCP observation | production typed event sink；不拥有 execution decision |
| PASS/FAIL manifest | Gate aggregator |

Phase 1 不能拥有两套 image type、两种 request target constructor、两套 scalar executor 或两种 verifier seal。
若 Phase 0 handoff 保留薄 proof stage，它是 executable-image construction 的内部步骤，不向 request/VM 暴露
第二 authority。

## 6. 两条线和角色合同

Phase 1 稳定存在的是 Development Line 与 Proof Line；实际 Agent/task ID 由 MAP1 决定。

| Role | Owns | Must not do |
| --- | --- | --- |
| 主 Agent / Integrator | MAP1、critical-path 调度、监控/接管、机械合流、freeze、receipt handoff | production/Proof 写入、最终 verdict、merge-time 语义修复 |
| Containment Development Agent | 一个 K0 fail-closed lane | 扩大 accepted support surface、修改 Gate 标准 |
| Executable Kernel Agent | K1 exact linker-image-entry-VM admission kernel | 为并发拆出第二 authority、验收自己 |
| Production Development Agent | 一个不重叠 compiler/validator/loader/VM/budget/request lane | 修改 K1 authority 或 Proof verdict |
| Contract/Negative Proof Agent | T-C/T-R canonical expected-red、contract、negative scenarios | 修改 production 使测试通过 |
| VCP Proof Agent | VCP-1 fixture/harness using production seam | 手工构造 internals、生成 PASS |
| Gate Proof Agent | selector、aggregator、manifest/checker、Gate self-tests | 改 production execution 或硬编码 scenario verdict |
| Acceptance Agent | frozen candidate read-only review、Gate execution、receipt | 修改 candidate、修复、合流或更新状态 |

Clarification、Design 和专项 Design Reviewer 都是 §8 条件 task，不预建固定角色。设计者可以随后实现自己的
决定；不能独立 review/accept 自己。强制隔离只有：

- Acceptance Agent 必须是此前没有写 candidate production/test/Gate 的全新 Agent；
- Proof Agent 不修改 production code 制造 PASS；
- Executable Kernel 始终只有一个 write owner；
- production Agent 可以写局部 unit tests，但不能拥有 canonical VCP/Gate verdict；
- integrator 不在 merge 时补默认值、bypass、equivalence 或第二 API；
- 资源不足时降低 leaf 并发，不取消独立 Acceptance。

## 7. MAP1、worktree 和调度

### 7.1 MAP1 first commit

派发首批 Development/Proof task 前，主 Agent 在独立 integration worktree 创建并单独提交：

```text
doc/implementation/bytecode-vm-convergence/tasks/phase-1-execution-map.md
```

MAP1 v0 至少记录：

- exact baseline commit/tree、Phase 0 input hashes 和 clean receipt；
- Phase Contract identity、两条线首批 ready frontier 和 join condition；
- 每个实际 Agent/task ID、role、input commit、branch/worktree 和 write/read set；
- `started_at`、`status_after`、预计产物、splittability 和 takeover 方法；
- first non-document commit、first executable proof attempt checkpoint；
- conditional Clarification/Design 的触发条件、integration order 和 candidate epoch。

main checkout 始终停在 `main`。每个并发 write owner 使用不同 worktree，直接建立在
`/Users/geek/workspace` 下；只读 Agent 可以共享 detached baseline worktree。实际路径不写死在本文。

worktree 与 task 不强制一一对应。默认按长期 write-set lane 分配 worktree：前一 task 已完整提交、handoff 且
worktree clean 后，同一 role/owner 的后续串行 task 可以复用；发生并行拆分、owner 冲突或新 candidate 隔离时必须
新建 worktree。takeover 只有在原 write Agent 已停止并记录未提交状态后才能接管原 worktree，否则新建 worktree 从
最后可信 commit 重做。MAP1 记录每次创建、复用、接管和释放。

### 7.2 Event-driven scheduling

任一 Agent 完成、失败、被中断或提交 commit 后，主 Agent 立即：

1. 验证 handoff 与 write set；
2. 更新 MAP1；
3. 重算 ready frontier；
4. 派发全部无依赖/写冲突的任务；
5. 对满足 join 的 commit 机械合流；
6. 必要时开始新 decision/candidate/evidence epoch。

不等待同一“批次”全部结束。只要某个 downstream 的全部依赖已经完成，就立即派发。

### 7.3 Watchdog and takeover

超过 MAP1 `status_after` 且没有可信产物时，主 Agent 询问完成内容、当前假设、blocker、可提交部分和剩余步骤。
根据回答自主选择：短 checkpoint、正常继续、条件澄清/设计、要求部分提交后结束、interrupt 或 takeover。

- 同一 worktree 始终只有一个 write Agent；
- takeover 可以是一个新 write owner 加多个 read-only diagnostic Agent；
- 可拆任务只有在 write set 和 semantic owner 同时可分时才并行拆开；
- 中央 K1、VM frame state machine 或 Gate verdict 不因 Agent 超时拆成多个 authority；
- Agent replacement、并发调整和 worktree 变化只更新 MAP1；共享 support/authority/VCP/Gate 选择按 §8 处理。

### 7.4 Commands

- Development Agent 只运行 task contract 指定的 focused command；
- 不自行运行全仓 `scripts/verify` 或无关 crate tests；
- 预计超过 30 秒的命令重定向到临时日志并可轮询；
- 暂不强制所有 Cargo 命令使用统一 hard timeout；MAP1 记录观察和接管时间；
- integration 可以运行最小 VCP/contract preflight，但不能产生 acceptance；
- Acceptance Agent 必须完整运行 canonical Phase 1 Gate；中断等于未运行。

MAP1 默认把 ordinary task 的首次 checkpoint 设在 30 分钟内，并以 45 分钟内出现首个非文档 commit、90 分钟
内出现首次 executable VCP/Gate attempt 为重排信号。未达成时报告真实 blocker并重排，不能继续扩写文档满足
进度。Clarification checkpoint 应更短。

### 7.5 Task handoff

每个非只读 task 的交付必须是一个可独立合流的 commit，并附：

- exact input commit 和 output commit/tree；
- 实际修改文件与 write-set 偏差；
- task contract 中每条 obligation 的 disposition；
- 已运行的 focused commands、退出状态和日志位置；
- 未运行项、已知 blocker、remaining risk 和建议的下一 ready task。

任何 Agent 不直接写其他 Agent 的 worktree，也不把未提交修改作为下游 task 的隐式输入。Clarification 默认
交付短 answer/citations，不要求 commit 或长报告。

## 8. 问题路由：Clarification 与 Design

Phase 1 不建立 BAUD1–BAUD7 全量审计 frontier。Phase 0 accepted receipt 已是共同 baseline；开发和 Proof owner
只读取自己 task 所需代码并立即产出代码/expected-red。

### 8.1 Clarification 条件

只有一个明确当前事实无法从 accepted handoff 或当前 owner 的正常代码阅读中得到，并正在阻塞 task 时派发。
可能的问题包括：某 current ingress 是否仍绕过 image pin、某 public constructor 的实际调用方、某 production
event 是否已存在。任务必须包含一个 question、consumer、exact baseline、短 checkpoint 和 citations 输出；
禁止目标 API、迁移顺序、Phase task 或 verdict。

Clarification 只阻塞消费者，不形成全局 join；默认不写仓库长文档。

### 8.2 Design 条件

只有存在尚未决定的目标选择，并且它影响多个 write owner、两条线共同合同、authority/ownership/failure
语义、公共/持久边界或难以撤销时，才派一个 narrowly scoped Design task。

Phase 1 的典型触发点是：Phase 0 handoff 无法唯一决定 executable image/entry 类型、K1 atomic boundary 或
budget terminal owner。私有 helper、局部数据结构和容易撤销的 lane 实现由 Development Agent 自己决定；
scenario/assertion/Gate carrier 由 Proof Line 决定；Agent/worktree 由主 Agent决定。

高风险 shared decision 由一个未参与该决定的 reviewer 在 dependent join 前审查。`FAIL` 只阻塞消费者，其它
K0/Proof task 继续。设计者可以随后实现，不得 review/accept 自己。

## 9. Implementation DAG

```text
MAP1 + Phase Contract preflight
  ├─ Development Line
  │    {K0A compiler, K0B image/opcode, K0C request/route containment}
  │      -> K0 containment receipt
  │      -> K1 executable-image/admission kernel
  │      -> {L1 compiler, L2 structural admission}
  │      -> {L3 loader/link/cache, K2 VM, L4 budget, L5 request, O1 conditional events}
  │
  └─ Proof Line（从第一批并行启动）
       {T-C/T-R expected-red, V1 VCP harness, G1 Gate/checker self-tests}
       -> rolling contract/negative/VCP/regression evidence

conditional Clarification/Design feeds only affected nodes
  -> rolling joins + minimum executable proof after each relevant join
  -> I1 merged integration proof
  -> F1 frozen candidate
  -> A1 independent acceptance
  -> R1 result and Phase 2 handoff
```

调度说明：

- K0A/B/C 是 Phase 1 第一个 production frontier，三项 write set 独立时全部并发；
- T-C/T-R、V1 和 G1 与 K0 从第一批并行，不等待完整设计或审计；
- K0 receipt 之前不得把任何 scalar expansion 合入 integration line；
- K1 是单一 authority kernel，不为并发拆给多个 Agent；
- L1/L2 若只依赖 Phase 0 accepted artifact contract，可与 K1 并发开发，但合流满足 exact producer-consumer join；
- L3/K2/L4/L5 只有 K1 API 完整提交后才 ready；
- O1 只有 executable Proof 表明 existing events 无法证明某个 required fact 时启动；
- Proof Line 不得使用 placeholder、test-only seam 或硬编码 PASS；
- integration 是串行 merge owner，不是最后补语义的 checkpoint。

## 10. Production task contracts

### K0A/K0B/K0C — Capability containment

共同 invariant：Phase 1 以外 capability 在进入错误运行语义前由唯一、可观测边界 fail closed。

- K0A：compiler/source admission 拒绝 Phase 1 之外 constructs/effects/value shapes；
- K0B：executable entry closure 拒绝 unsupported opcode、target、effect、type/shape；
- K0C：request/route admission 只允许 accepted unary deployment/entry，拒绝 stream/task/child/host lanes。

K0 不得：

- 通过字符串、request mode 或 package 名称猜 capability；
- 在一个边界失败后允许另一个 fallback executor；
- 禁用 Phase 1 accepted fixture；
- 把当前危险路径仅记录为 later follow-up。

K0 receipt 必须由独立 negative test lane 证明每个 gate 的 exact error owner 和无 fallback route。

### K1 — Executable image and VM admission kernel

一个 Agent 原子拥有 Phase 0 accepted handoff 定义的 linker output、immutable image、entry pin 和 VM input
协议。必须：

- 删除 broad verifier/seal 的 execution-authority 角色；
- 若有薄 proof stage，仅保留 Phase 0 handoff 明列 checks，且输出不成为第二事实源；
- exact deployment owner、package closure、operation entry 和 executable image 不可替换；
- request/VM 不能读取 raw candidate、ambient artifact root 或 unchecked entry；
- test crate 不能直接构造 linked/image/VM entry internals；
- publication/cache failure 不发布半 image；
- K1 public surface 足够窄，使 Phase 0/1 VCP 只能走 production composition seam。

K1 允许跨 crate write set，因为这些类型必须作为一个原子协议演化；不能用 interface-only placeholder 提高并发。

### L1 — Compiler scalar/local producer

- 只发射 Phase Contract accepted source constructs/opcodes；
- exact direct-local target、arity、scalar type、frame/slot、source/statement facts由 source/lowering 提供；
- unsupported shape/effect/target 在 emission 前 fail closed；
- fixture 至少包含 helper local call、scalar slot、arithmetic、comparison/branch 和 return；
- 不从 initializer/syntax/string binding 重新推导 runtime authority；
- 不顺手启用 aggregate、throw、host、tail、generic 或 `InOut`。

### L2 — Bounded structural admission

- 在 linker 访问 artifact-controlled index 前完成 bounded decode/index/offset/count checks；
- exact schema/ISA/registry pins 和 artifact content identity；
- malformed word/operand/jump/table/constant/source row fail closed；
- limits 使用 production policy，不在 VCP 使用 `u64::MAX` 代替边界证明；
- structural validator 不猜 source type/effect/target，也不承担 Phase 1 semantic allowlist 的第二实现。

### L3 — Exact loader/linker/cache/route

- 从 release/deployment owner 加载 exact package closure；
- relocation、registry、entry 和 signature exact resolution；
- cache key 使用 exact deployment owner，失败不发布 partial image；
- request 执行不重读 ambient artifact root/release state；
- no first package/operation、type equivalence、std binding bypass 或 fallback；
- production route 使用 K1 唯一 composition/admission API。

### K2 — Synchronous VM scalar/local core

- fixed-width immediate scalar slot semantics；
- scalar load/store/move/copy 不借用 aggregate lifecycle 猜测；
- branch/PC/operand state exact；
- local call push frame并在同一 dispatch loop 继续，不递归 Rust evaluator/future；
- return 移入 caller destination并截断 frame/value segment；
- frame/slot/operand bounds 在 admission/runtime 双边受控；
- unsupported opcode/control 返回稳定 fail-closed terminal；
- 不执行 aggregate drop/COW、ordinary unwind、Pending 或 resource path。

如果 immediate scalar copy 仍会调用错误 aggregate lifecycle sidecar，K2 必须修复 scalar-specific closure 或缩小
surface；不能提前实现 Phase 2 aggregate executor。

### L4 — Raw fuel、deadline 和 internal stop

- VM 私有 dispatch wrapper 对每次尝试先调用一次 `VmBudget::before_dispatch`，成功后相邻且恰好调用一次
  `dispatch_one`；中间无 yield、callback、poll 或 fallible bookkeeping；
- `before_dispatch` 在唯一 request-owned `ExecutionBudget` 锁内原子授权并增加一个 raw unit；随后 instruction
  error 仍计费且不可重试；不存在 quantum grant/precharge/refund/remainder/dispatch token 或 VM fuel counter；
- raw executed count 与 semantic attribution 分账；semantic charge 不消耗 raw capacity，O1 charged count 只从 raw
  count 派生；
- first/segment/verified-loop poll 与 raw cadence 都由 budget 的 authoritative raw counter 决定；
- hard limit `N` 允许前 N 次 dispatch，只有 N+1 失败；`u64::MAX` 必须证明 `MAX-1 -> MAX -> N+1 fuel`，raw
  overflow 不可达，semantic/poll overflow fail closed；
- deadline/completion/cancel/session-stop 在同一个 winner cell 竞争；每次 open transition 取得 budget lock 后才从
  budget-owned trusted monotonic clock 取时，due deadline 优先；
- hard limit exhausted 产生唯一 terminal，当前 frame 不可 catch/继续；
- artifact、test fixture 和 host adapter不能关闭、扩大或重置 limit；
- supported scalar opcode 单次工作量由 Phase 1 admitted scalar/frame limits 界定。

### L5 — Unary request boundary and terminal

- production route 解析 exact deployment和 operation entry；
- request target 只能来自 K1 production image pin；
- deterministic unary scalar response 使用 canonical boundary carrier；
- success/VM failure/deadline/internal stop 只选择一个 terminal；
- request row 以 typed `(RouterSessionEpoch, RequestId)` 为 key；activation 精确返回 `Activated`、
  `RevokedByCancel`、`RevokedBySessionStop` 或 `Invalid`，两个 revoked outcome 都只执行一次
  `StopWithoutResponse`，不结算 budget、不发 terminal、不创建/重复 cleanup；`Invalid` 使用 admission error
  `bytecode request reservation activation failed`，同样不创建 budget/inventory/terminal/cleanup；
- synchronous closure 不创建 Pending owner、stream/resource/child state；
- request cleanup 可由 typed event 观察；
- unsupported request/capability 在 K0C fail closed，不进入 adapter fallback。

## 11. Test development、VCP 和 Gate

### 11.1 Canonical test lanes

Proof Line 从第一批 task 起至少覆盖两个可独立写入的 lane：

| Lane | Coverage |
| --- | --- |
| T-C | compiler -> artifact -> structural/link contract；unsupported source/malformed artifact/exact identity |
| T-R | image entry -> VM -> budget -> request；frame/local/branch/return/terminal/containment |

生产 Agent 可保留局部 unit test，但 T-C/T-R 是 canonical contract/negative owner，不能由对应 production Agent
自行宣告完成。

### 11.2 VCP-1

VCP-1 使用 Phase 0 accepted production composition seam：

```text
real .skiff fixture
  -> production compiler
  -> canonical immutable artifact publication
  -> production deployment load/admission/image cache
  -> exact route + entry pin
  -> production request entry
  -> synchronous VM local-call execution
  -> deterministic scalar response 3.0
```

Fixture 至少真正执行：scalar slot、`helper(2)` exact local call、arithmetic、一个 branch/comparison 和 return；
不能因简化 fixture 让其中任一 accepted behavior 只剩 unit test。

Raw production events 至少包含：fixture/artifact/deployment identity、image admission、route/entry、function/frame
entry、actual VM dispatch、local call/return、budget poll/count、response terminal 和 request cleanup。Harness 不生成
PASS manifest，不直接调用 linker/image/VM constructors。

### 11.3 Required negative/boundary scenarios

Gate 至少运行：

1. malformed/corrupt artifact 在 bounded production admission 失败；
2. wrong deployment/package/entry 不会 first-match 或 ambient fallback；
3. unsupported source construct 在 compiler gate 失败；
4. unsupported opcode/target/effect 在 image capability gate 失败；
5. non-unary/host/stream/task request 在 request gate 失败；
6. raw fuel exact-boundary success 和 exhausted terminal；
7. deterministic deadline/internal-stop poll；
8. deep local calls不递归 native evaluator stack，且受 frame/fuel limit；
9. no Pending/resource/child owner and terminal exactly once；
10. Gate self-tests拒绝 dirty/stale/missing/zero/skip/tampered evidence。

### 11.4 Phase 1 Gate

唯一 canonical Gate command 由 Proof Line 实现并注册进 `scripts/verify.mjs`。它聚合：

| Evidence class | Required proof |
| --- | --- |
| focused/unit | compiler、structural、link/image、VM、budget、request local behavior |
| producer-consumer contract | source fact 到 executable/VM 没有丢失、重建或宽松 join |
| containment | 所有 disabled lane 在唯一 gate fail closed |
| VCP-1 | production-shaped exact route 到 scalar response |
| negative/lifecycle | malformed、mismatch、limit、terminal、cleanup |
| structural/reverse search | 无 seal authority、fallback、alternate executor、public unchecked constructor |
| budget/bounded work | raw fuel/poll/frame/scalar work limits |
| regression | Phase 0 accepted VCP/Gate 和 Proof Line 滚动记录的既有 selectors |

Gate 必须使用 Phase 0 accepted evidence infrastructure：detached clean worktree、exact commit/tree、binary hash、
durable raw logs/events/manifest/receipt、非零 scenario、无 skip、无跨 epoch 拼接。Manifest 由 Gate aggregator
从 raw evidence生成。

不默认要求笼统 full repository verify。Proof Line 在 candidate freeze 前闭合 required selector matrix；required
command 中断或既有失败都是 FAIL，除非 freeze 前已有 exact baseline waiver、owner、expiry 和不受本 Phase
影响的证据。测试矩阵随真实状态转移滚动补充，不能等 production 实现结束后才第一次建立。

## 12. Integration、freeze 和 acceptance

### 12.1 Integration order

唯一 integration line 按实际 ready frontier滚动合流，不等待两条线各自“全部完成”。默认依赖是：

1. MAP1/Phase Contract receipt；
2. K0A/B/C 与 T-C/T-R expected-red、V1/G1 skeleton 各自 ready 即合流；
3. K0 containment receipt 后才合流 scalar expansion；
4. K1 完整原子 commit；
5. L1/L2 及其 contract evidence；
6. L3/K2/L4/L5/O1 与对应 negative/VCP evidence滚动 join；
7. merged-state full VCP/Gate preflight；
8. frozen candidate。

每个 join 后立即重算 ready frontier并运行受影响的最小 contract/VCP preflight。Integrator 不为合流增加 type
equivalence、adapter、feature bypass、默认值或第二 API；出现冲突退回原 owner。

### 12.2 Frozen candidate

freeze receipt 至少记录：

- exact commit/tree、clean status、worktree；
- 所有 task commits 和 integration order；
- compiler/test/runtime binary hashes；
- artifact/registry/schema identities；
- required Gate selector matrix；
- 未接受 capability 状态。

freeze 后任何 code、test、fixture、Gate、event 或 manifest schema 变化都开始新 candidate/evidence epoch。

### 12.3 Independent acceptance

全新 Acceptance Agent 只接收：Phase contract、Phase 0 receipts、frozen candidate commit/tree、canonical Gate
command 和 durable evidence output location。它不接收开发者的 PASS 总结。

Acceptance 必须：

1. 只读审查 candidate 是否满足 Phase Contract 和所有实际触发的 shared decision receipts；
2. 专门寻找手工 image/target、硬编码 evidence、fallback 和 unsupported reachability；
3. 在 detached clean worktree 运行完整 Gate；
4. 核对 raw evidence 与 manifest，不只看 exit code；
5. 第一行给出 `PASS` 或 `FAIL`；
6. 记录 exact candidate、tree、commands、counts、hashes、waivers 和 findings。

Acceptance Agent 不修复。`FAIL` 由主 Agent退回对应 owner；修复后重新 freeze，并由新的 Acceptance task 验收。

## 13. Acceptance checklist

Phase 1 只有全部成立才为 `accepted`：

- [ ] Phase 0 closure inputs属于同一 valid epoch；
- [ ] MAP1 在首次派发前单独提交并记录两条线、实际 Agent、worktree、超时、接管和 join；
- [ ] Development/Proof 首批 task 并行启动并产生非文档 code/evidence；
- [ ] Clarification 只回答具体事实问题，未形成全量审计 barrier；
- [ ] 只有满足条件的共享目标选择才产生 Design/review receipt；
- [ ] K0 containment receipt 先于任何 scalar expansion merge；
- [ ] accepted/disabled support matrix 在 compiler、image 和 request 三个 owner 一致；
- [ ] broad verifier/seal 不再是 execution authority；
- [ ] exact deployment/image/entry route 无 ambient reread、first-match 或 fallback；
- [ ] VM scalar/slot/branch/local/return 使用单一同步 dispatch loop；
- [ ] raw fuel、deadline/internal-stop 和 terminal semantics 通过 boundary scenarios；
- [ ] VCP-1 经过 production composition 并返回预期 scalar result；
- [ ] raw events证明 exact route、VM local call/return、budget、terminal 和 cleanup；
- [ ] unsupported lanes全部 fail closed，无 Pending/resource/child owner；
- [ ] Gate 聚合所有 required evidence class并拒绝 dirty/stale/missing/zero/skip/tampered；
- [ ] durable evidence 与 frozen candidate commit/tree/hashes 一致；
- [ ] 全新 Acceptance Agent 给出 PASS；
- [ ] Phase 1 result和 capability ledger只把本 Phase closed surface标为 accepted；
- [ ] Phase 2 输入、remaining disabled lanes 和 blocker 已明确记录。

## 14. Stop and escalation conditions

主 Agent 自主处理 Agent 数量、并发、worktree、任务拆分、重试、接管和合流顺序。以下情况停止并向用户提出
最小决策问题：

- Phase 0 accepted receipts 无法给出唯一 verifier/image/entry target；
- scalar/local closure 必须依赖 aggregate lifecycle、ordinary exception、host/Pending 或 cross-owner capability；
- K0 containment 会改变 canonical 用户可见语义；
- production composition seam 不能覆盖真实 route/entry 而需要新增 execution authority；
- K1 无法在一个原子 owner 中实现，必须形成长期双 image/seal；
- hard fuel/deadline terminal 与 canonical error语义冲突；
- required Gate 因外部权限/资源不可执行；
- 一个 shared design choice 经独立 review 后仍无法由现有 authority 消解。

不能通过扩大 Phase、降低 Gate、硬编码 manifest、合并冲突角色、保留 fallback 或“先跑起来以后再删”绕过。

## 15. Result and Phase 2 handoff

PASS 后创建：

```text
doc/implementation/bytecode-vm-convergence/results/phase-1.md
```

至少记录：

- baseline、Phase Contract、conditional clarification/design receipts、candidate、merge commit 和 tree hashes；
- MAP1 final revision、实际 Agent/task ID、worktree 和 takeover；
- K0/K1/L*/T*/V1/G1 commits；
- exact accepted opcode/type/entry/support matrix；
- verifier/image/VM admission最终 surface 和 reverse-search evidence；
- canonical Gate command、required selector matrix、scenario counts；
- durable raw evidence、manifest、acceptance receipt locator/hash；
- raw fuel/deadline/internal-stop limits 和结果；
- disabled capability ledger；
- Phase 2 value-lifecycle producer/consumer seam、first ready Development/Proof tasks、conditional questions 和
  unresolved risks。

只有 result commit 根据 valid acceptance receipt 合入 `main` 后，总计划才把 Phase 1 标为 `accepted` 并允许
Phase 2 production implementation。
