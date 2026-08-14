# Bytecode VM 架构收敛重构总计划

> Status: project plan; Phase 0/1/2/3/4 accepted; Phase 5 active in recovery epoch r1
>
> Created: 2026-08-12
>
> This document is implementation coordination, not a second semantic authority.

本项目修复当前 bytecode-only production path 的架构和运行语义。目标不是继续扩大 opcode 或 verifier
覆盖，而是先建立一个小、正确、可解释、可逐阶段证明的执行模型，再逐项恢复能力。

本项目采用[大型重构的滚动细化与多 Agent 实施原则](./large-change-execution-principles.md)。它明确替代
[`worker-crate-parallel.md`](../../worker-crate-parallel.md) 中“多个 Agent 直接写同一 main checkout”、
“interface stub 可先合入 main”及“只按直接接口持续启动下游实现”的规则。crate 仍可用于划分 leaf write
set，但不再作为语义任务或完成单位。

## 1. Authority 和输入

语义 authority 仍然是当前 `doc/reference/` 与 `doc/architecture/`。本项目的直接输入是：

- canonical VM architecture：[`bytecode-vm.md`](../../architecture/bytecode-vm.md)；
- current implementation review：
  [`bytecode-vm-architecture-review.md`](../../architecture/bytecode-vm-architecture-review.md)；
- compiler、runtime、boundary、deployment、Actor 和 testing 相关 canonical 文档；
- 当前生产代码和 Git 历史。

原 [`doc/implementation/bytecode-vm/`](../bytecode-vm/) 目录是第一次实施的历史证据和需求清单，不再
控制本次重构的 phase acceptance。它记录的遗漏不能被静默丢弃，但其旧 phase 状态不能证明当前能力
完成。

文档优先级为：

```text
user-visible reference / canonical architecture
  -> 本项目总体计划
  -> 当前 accepted Phase Contract / shared decision receipts
  -> task contract
  -> implementation
```

发生冲突时停止实现并回到上一级，不在下游增加兼容推断。

## 2. 当前判断

当前生产环境已经 bytecode-only，旧 tree evaluator、`RuntimeAssembly` 执行栈和 production fallback 已被
删除。因此本项目没有第二次 engine cutover，也不恢复旧 evaluator。

当前主要风险不是“某些能力尚未实现”，而是：

- 已可达路径中的 value lifecycle、writable path、exception、HTTP/Pending、stream/resource 等语义错误；
- compiler、artifact、linker、残留独立verifier与request adapter之间存在需要hard cut的多重authority和宽松
  fallback；
- scheduler / heap / request API 无法表达跨 owner、完整 root 和 session lifetime；
- 性能、fuel 和 memory budget 不能界定真实工作量；
- 第一次实施的 phase gate 没有约束 production admission 和 deletion。

残留 verifier 处在错误位置：它经常替上游恢复缺失的 type、effect、target 和 lifecycle fact，同时又给
下游一个“已经验证”的 seal。Phase 0 曾要求关闭其去留决定；Phase 5 的 DEC1/Contract Amendment r2 已最终
裁定删除独立 production verifier crate/stage及其facts/seal。保留的只有pre-link bounded structural validation、
single atomic image construction内的private finite structural closure、exact linking与执行层必要的checked runtime
invariants；不存在可独立调用、配对、缓存或兼容转发的“薄verifier”，也不得重推source-或registry-owned facts。

### 2.1 Git 历史显示的失败机制

这次问题不是某一个实现者漏写了几个分支，而是实施顺序允许“每层都局部完成，组合后仍然错误”：

1. 2026-08-09，旧 Phase 1 先完成 artifact schema、decoder 和 structural validator。它的
   [result](../bytecode-vm/results/phase-1.md)明确记录 Live manifest 的 engine 仍是 `legacy-tree`，并注明这只是
   全栈回归、不是 bytecode 执行证明。也就是说，新链路可以在没有被执行的情况下获得 phase completion。
2. 2026-08-10，从 `527afafb`（seal verified images）到 `e1bb85f6`（freeze v5 authority pins），大量提交先后
   freeze/prove schema、handoff、linker、verifier 和 authority pin；真正的同步 VM core 到 2026-08-11 的
   `cde272ff` 才出现。producer 与 consumer 没有在同一 Semantic Closure 中共同演算，接口外形因此先于
   可执行语义成为事实。
3. 2026-08-11，scalar vertical slice（`8a122117`）出现后，同一天继续扩大 compiler、VM、heap、scheduler，
   随即以 `e3c601dd` 强制 bytecode-only admission，并以 `a761a989` 删除旧 evaluator。能力扩张、语义修复、
   cutover 和 deletion 被压在同一个连续窗口，没有逐能力 frozen-candidate receipt。
4. 随后的 stream、HTTP、error 和 verifier 工作继续按 crate/边界推进；局部测试能证明类型和表格自洽，却
   不能证明真实 ownership、Pending、drop、unwind 或 provider routing。直到本次跨层代码审查，多个局部
   PASS 才被组合成系统级反例。

因此根因是四个机制叠加：按 crate/阶段交付代替语义闭环、真实消费者出现得太晚、Gate 没有绑定新执行
路径、旧路径在 replacement 分能力验收前删除。verifier 随后自然变成“语义债务汇集层”：上游没有传递的
事实由它猜，下游再把 seal 当作完整保证。单纯给 verifier 增加规则只会加固这个结构。

本计划据此改变顺序：先冻结精简 Phase Contract，然后 Development Line 与 Proof Line 并行；expected-red、
VCP/Gate 和最小 kernel 尽早互相反馈。每个 Phase 只在同一 frozen candidate 的新路径上通过 Gate 后扩张支持面。
crate 只负责 write-set 隔离，不再提供完成语义。

### 2.2 Review finding 的 Phase owner

| Review finding | 首要 owner Phase | 接受前状态 |
| --- | --- | --- |
| VM-06/07/11：authority、fallback、image/admission | Phase 0/1 | 最小链路修复，其余能力 disabled |
| VM-01/02：value lifecycle、drop、writable path/COW | Phase 2 | fail closed 或 containment |
| VM-03：exception envelope、catch identity、unwind | Phase 3 | fail closed |
| VM-04/12/13：Pending、root graph、session/request lifetime | Phase 4 | 禁止新增 async lane |
| VM-04/05：HTTP、ResourceRef、stream ownership/backpressure | Phase 5 | containment 或 disabled |
| VM-08/09/10：task/cross-owner heap、materialization、handle provenance | Phase 6 | 分 lane disabled |
| VM-14：fuel、memory、hot-path bound | Phase 1 建基础，Phase 7 完整收口 | 不宣称有统一预算保证 |

Phase 0 必须用当前代码证据校正这张初始分配表；只有具体事实不清楚时才派 Clarification task。发现某项是
更早 Phase 的前置条件时，只能前移或缩小支持面，不能把错误可达路径留给最终 Gate。

## 3. 目标执行拓扑（Phase 0 待冻结）

当前 working direction 是：

```text
source-owned semantic facts
  -> lowering / bytecode emission
  -> relocatable artifact
  -> bounded structural decode and validation
  -> exact deployment linker
  -> immutable ExecutableBytecodeImage
  -> synchronous VM core
  -> scheduler / typed effect adapter when actually needed
  -> request boundary result
```

职责原则：

| Fact / behavior | 唯一 owner 方向 |
| --- | --- |
| source type/effect/lifecycle/loan/capability fact | source analysis / lowering |
| persistent opcode/operand/schema limits | artifact model + structural validator |
| exact deployment/package/target/registry resolution | loader/linker/image construction |
| frame、instruction、local call、unwind execution | VM |
| physical share/move/drop/COW | VM lifecycle executor + heap primitives |
| Ready/Pending/park/wake/cancel/deadline | scheduler |
| native ABI/effect identity | typed canonical registry |
| provider state/cancel/drop | ResourceTable entry |
| request/session lifetime | request supervisor bound to exact runtime session |

Phase 0 必须把此表变成精确代码 owner 和 API 边界。任何 fact 缺失时，producer/emission/link/admission 在
唯一位置 fail closed；linker、validator、VM 或 adapter 不得从字符串、类型外形或默认 package 重建。

## 4. 项目完成定义

本项目完成不是“所有 opcode 都有 match arm”，而是：

1. 每个声明支持的 capability 状态为 `accepted`，拥有 frozen-candidate VCP 和独立验收；
2. 尚未支持的 capability 在一个明确边界状态为 `disabled`，不会进入错误运行语义；
3. compiler 到 runtime 的每种 fact 只有一个 authority；
4. 不存在 std/package 特判、registry mismatch bypass、宽松 type equivalence 或 test-only execution seal；
5. 所有真实等待只表现为真实 Pending，不阻塞 VM/Tokio worker 后伪报 Ready；
6. logical value copy/move/drop、path mutation、throw/unwind 和 cross-owner materialization 可端到端解释；
7. request/session/child/resource/root graph 完整且 terminal exactly once；
8. fuel、memory 和 hot-path 工作量有可执行上限；
9. whole-system acceptance 只汇总已接受能力，不在最后一阶段首次实现语义。

## 5. 能力状态

本项目统一使用：

| 状态 | 含义 |
| --- | --- |
| `accepted` | 已在 frozen candidate 上通过 VCP 和独立验收 |
| `enabled-unaccepted` | 当前可达但缺少正确性证明，必须优先 containment 或修复 |
| `disabled` | 在唯一入口明确 fail closed |
| `planned` | 未实现且不应可达 |

Phase 0 建立当前 capability ledger。每个后续 Phase 只允许把自己拥有的 capability 从
`enabled-unaccepted` / `disabled` 迁移为 `accepted`；不能顺手开放下一 Phase 的能力。

## 6. 垂直闭环证明

从 Phase 1 开始，每个 Phase 必须在两条执行线启动前定义一个
**垂直闭环证明（Vertical Closure Proof，VCP）**。VCP 从本 Phase 的真实 producer 或公开输入开始，
经过 production-shaped composition 到达本 Phase 的最终 consumer，并断言外部结果以及 exact owner、route、
Pending、drop 等关键事实。

VCP 不一定启动整个产品，因此不笼统称作 E2E；但它也绝不能是单 crate test 或直接构造 VM 内部对象。
详细规则见[原则文档 §7.1](./large-change-execution-principles.md#71-垂直闭环证明vertical-closure-proofvcp)。

Phase 0 必须找到或建立 Phase 1 VCP 的真实 composition seam。Phase 1 的可执行 harness 由 Proof Line 从首批
task 开始实现，与 Development Line 并行；不得等生产实现结束后才首次整合。seam 当前事实不明时启动
Clarification；若新增 seam 会改变 execution authority，才启动条件 Design task。

### 6.1 VCP 与 Phase Gate

VCP 是 Phase Gate 的必要证据，不是 Gate 本身：

```text
Phase Contract
  -> Proof Line: tests / fixtures / scripts / observability
  -> VCP evidence manifest
  -> Phase Gate 聚合 VCP + focused + negative + structural + regression evidence
  -> independent acceptance verdict
```

VCP 和 Gate 都必须可执行。Phase 0 要为 Phase 1 决定一个注册在仓库验证图中的 canonical selector；它必须
运行真实 fixture、生成并校验 manifest、拒绝 skip/零场景/stale candidate evidence。最终 result 文档只引用
该命令和 evidence，不替代它们。

### 6.2 测试是并行 Proof Line

每个 Phase 在首批 production task 同时启动独立 Proof Line。Phase Contract 先给出最小外部成功结果、一个
fail-closed companion、production seam 约束和不得伪造的事实；Proof Line 先落成 expected-red harness，再随
真实状态转移滚动扩充 scenario matrix，而不是等待实现完成后补 happy-path unit test。

最低 evidence 层包括：

- local/focused algorithm tests；
- producer-consumer contract tests；
- VCP；
- malformed/unsupported/race/cancel/drop 等 negative/lifecycle matrix；
- no-fallback/no-bypass/no-second-authority structural checks；
- 本 Phase 声明涉及的 budget/performance checks；
- 与此前 accepted capability 的 regression selector。

具体规则见[原则文档 §7](./large-change-execution-principles.md#7-vcp测试与-gate)。

每个 accepted Phase 的 VCP 和 negative matrix 会成为后续 Phase 的永久 regression selector。测试覆盖按
semantic support surface 累计，不按 crate 测试数量统计；同一 scenario 只有一个 canonical owner，避免
重复 fixture 掩盖真实 transition 缺口。

## 7. 初步 Phase DAG

下面只冻结依赖方向、Phase 目标和初步 VCP。除 Phase 0 外，具体接口、task DAG 和命令均在前一 Phase
接受后滚动细化；实际 Agent、branch、worktree 和派发顺序在执行每个 Phase 时由滚动 Execution Map 决定。
执行中发现一个 Phase 无法形成单一 Semantic Closure 时，可以拆分 Phase，但不能绕过前置 acceptance。

### Phase 0 — Architecture reset and validation foundation

关闭 verifier disposition、目标 pipeline、authority map、Phase 1 MVP、capability containment 和 Phase 1
VCP。补充闭合任务从共同 Phase Contract 同时启动可执行 Proof Line，产出 Phase 1 两条线的 first ready handoff；
Clarification/Design 只按具体问题条件启动。

详细计划：[`phases/phase-0-architecture-reset.md`](./phases/phase-0-architecture-reset.md)。
执行结果：[`results/phase-0.md`](./results/phase-0.md)。
补充闭合任务：[`tasks/phase-0-supplemental-closure.md`](./tasks/phase-0-supplemental-closure.md)。

### Phase 1 — Trusted synchronous core

建立最小可信同步执行链：source -> artifact -> structural validation -> exact link -> executable image ->
scalar/local-call VM -> unary response。删除这条链上的 fake seal、type/target fallback 和手写 admission
bypass。Phase 1 以外的 lane 必须 fail closed。

详细计划：[`phases/phase-1-trusted-synchronous-core.md`](./phases/phase-1-trusted-synchronous-core.md)。

初步 VCP：一份真实 `.skiff` unary fixture 经 production compiler、artifact store、loader/linker、VM 和
request response 返回确定 scalar 结果；manifest 同时证明 exact build、entry 和 VM dispatch，损坏 artifact
在真实 admission boundary 被拒绝。

### Phase 2 — Value lifecycle and writable path

闭合 exact lifecycle fact、VM lifecycle executor、heap share/move/drop、aggregate COW、owned replacement
root 和 overwrite/return/tail/unwind drop。删除 emitter lifecycle 猜测和事后 frame reconciliation。

初步 VCP：source aggregate snapshot -> copy/argument/container/return -> nested mutation；外部结果证明 alias
隔离，内部事实证明 share/COW/drop，missing plan 稳定拒绝。

### Phase 3 — Outcome and unwind

统一 opaque exception envelope、actual catch identity、return/throw/VM failure/platform terminal、region cleanup
和跨 resume rethrow。Phase 3 不接新 host capability。

初步 VCP：source throw/catch/rethrow 与一个 cleanup owner 经过真实 compiler/linker/VM/request path；同一异常
在同步和受控 resume 后保持 identity，terminal 只选择一次。

### Phase 4 — Scheduler, Pending and request ownership

闭合 Ready/Pending、park/wake/claim、cancel/deadline race、suspended invocation roots、session-owned request 和
child control frame。先用 deterministic controlled completion，不同时接真实 HTTP。

初步 VCP：真实 VM effect site 经 production scheduler 返回 actual Pending，完成或取消后恢复原 site；证明
一次 publish/wake/claim、完整 owner transfer 和 session disconnect terminal。

### Phase 5 — Typed host effects, resources and streams

建立 typed registry-to-executor bridge、ResourceTable-owned provider state、真实异步 HTTP 和 bounded
stream/backpressure。删除 adapter singleton、字符串 dispatch、blocking `join` / `recv`。

初步 VCP：隔离 runtime 中 deterministic host server 分别覆盖 Ready、Pending、timeout/cancel 和两个并存
stream handle；证明 handle 精确路由、bounded buffer、drop/cancel 和无 worker 阻塞。

Router 侧 owner：Phase 5 的 stream VCP 与 Phase 7 的 whole-system 会触及 router 的 WS→HTTP chunked 传输
路径；两个 Phase 的 contract 必须显式列出 router 侧 write owner 与其真实 composition 边界。

### Phase 6 — Cross-owner execution and managed-memory readiness

闭合 child owner/heap/budget/boundary materialization，再按独立 lane 逐项开启 service、task、interface、callback
和 Actor。完整 pending/root graph 通过后才能启用 request GC/compaction；必要时本 Phase 拆成 6A/6B。

初步 VCP：caller 和 provider 使用不同 exact owner/heap，经 production child trampoline 传参、返回和普通
throw；证明无 raw handle 穿越、parent 同步恢复和 Pending chain root 完整。

拆分标准（提前定死）：6A = cross-owner heap + boundary materialization + 第一个 lane（service）的 VCP；
6B = 其余 task/interface/callback/Actor 按 per-lane gate 逐个开启；request GC/compaction 只在完整 root graph
闭合后开。统一 memory ledger 归本 Phase；此前的每个 heap/lifecycle 改动必须保持 per-request 内存上限可观测
（沿用 Phase 1 raw fuel 的先例）。

### Phase 7 — Whole-system closure, budget and final acceptance

这不是 cutover。旧 evaluator 已删除，production 已是 bytecode-only。Phase 7 只完成统一 memory/fuel/hot-path
门禁、observability、支持面清单和 whole-system acceptance；不能首次实现新的语言或 boundary 语义。

某个 whole-system scenario 暴露语义缺口时，重开原 owner Phase。最终 VCP matrix 组合此前 accepted receipts，
再运行真实 HTTP/service/stream/task/Actor 中实际声明支持的场景。

## 8. Phase 内核心流程

每个 Phase 按 [`runbook.md`](./runbook.md) 的 9 步执行；该文件是流程的唯一权威。Clarification/Design 的
触发条件见[原则文档 §4](./large-change-execution-principles.md#4-条件支持任务)。下一 Phase 可以提前做只读
调查，不能在前一 Phase 未 `accepted` 时启动 production implementation。

## 9. Worktree 和 Agent 约束

实际拓扑由当前 Phase 的 Execution Map 决定。允许的典型形状是：

```text
/Users/geek/workspace/skiff-bcvm-pN-integration
/Users/geek/workspace/skiff-bcvm-pN-<leaf>
/Users/geek/workspace/skiff-bcvm-pN-gate
```

- main checkout 始终留在 `main`；
- read-only investigator 不需要各自的 worktree；main 无法保持 exact baseline 时，共用一个 detached
  baseline worktree；
- 每个并发 write owner 一个 leaf worktree；
- central kernel 不能为满足 crate 边界而拆给多个 owner；
- integrator 串行合流，不在 merge 时发明兼容语义；
- gate worktree 从 frozen commit 创建，由未参与生产实现的 acceptance owner 只读验收；
- candidate 任意变化开启新 evidence epoch。

Phase plan 只冻结角色分离、write set 和验收约束；实际 Agent、worktree 数量、路径和复用方式不在总体
计划中提前冻结。

## 10. 当前文档状态

| Document / Phase | Status |
| --- | --- |
| reusable execution principles | written; adopted by this project |
| project plan | active |
| Phase 0 | accepted; [`results/phase-0-closure.md`](./results/phase-0-closure.md) |
| Phase 0 supplemental closure | accepted; durable Gate receipt recorded |
| Phase 1 | accepted; [`results/phase-1.md`](./results/phase-1.md) |
| Phase 2 | accepted; [`results/phase-2.md`](./results/phase-2.md) |
| Phase 3 | accepted; [`results/phase-3.md`](./results/phase-3.md) |
| Phase 4 | accepted; [`results/phase-4.md`](./results/phase-4.md) |
| Phase 5 | active; Contract Amendment r1 + [`tasks/phase-5-execution-map.md`](./tasks/phase-5-execution-map.md) |
| Phase 6–7 | outline only; not implementation-ready |

Phase 0/1/2/3/4 均已由独立 Acceptance Agent 在 exact detached candidate 上通过 canonical Gate，且各 result
commit 已合入 `main`（`results/phase-*.md` 记录 accepted candidate、merge commit/tree 与独立 Acceptance
receipt）。Phase 5 从 exact Phase 4 main baseline 进入 recovery epoch r1；旧中断 lanes 全部只是 audit/salvage
source，不构成已完成成果。Phase 6–7 仍未授权 production implementation。

**稳定 dev env 运营注意**：本机常驻 dev 进程（router/runtime/各 client）仍在运行旧二进制，`main` 上
Phase 1–3 的 admission 收紧只影响**下一次重建并重启**。真实业务服务（aihub/registry/agine 的 string/stream/
host-effect 面）会在那次重启后 fail closed，直到 Phase 5/6 恢复对应能力。恢复路径：在 Phase 5/6 验收前，
dev 栈重建时把二进制钉在最后一个业务服务可用的 accepted Phase，或显式接受退化；不得为了恢复 dev env
临时放宽 admission。
