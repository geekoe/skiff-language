# Phase 0 补充闭合任务（重新执行）

> Status: ready for restart; previous closure attempt aborted and is not acceptance evidence
>
> Kind: Phase 0 recovery and executable proof closure; not Phase 1 production implementation
>
> Historical implementation: `5592c694`
>
> Historical merge: `01f33c2f`（与 `5592c694` tree 相同）
>
> Supersedes the execution workflow in the earlier revision of this file
>
> Authority: [`phase-0-architecture-reset.md`](../phases/phase-0-architecture-reset.md)、
> [`large-change-execution-principles.md`](../large-change-execution-principles.md)、
> [`bytecode-vm-architecture-review.md`](../../../architecture/bytecode-vm-architecture-review.md)

当前 Phase 0 已把一版审计、决策、VCP harness 和 selector 合流到 `main`，但原 acceptance 不满足
production-shaped VCP、candidate-specific evidence、Gate 聚合和独立验收合同。因此 Phase 0 仍是：

```text
implementation: integrated
acceptance: blocked
Phase 1 production implementation: not authorized
```

本任务只补齐 Phase 0 的真实执行证明和 acceptance closure。它采用一条 Development Line 和一条 Proof Line；
Clarification、Design 和专项 Review 只有遇到具体问题时才启动，不再先做六份全量审计和完整设计瀑布。

## 1. 已中止尝试的处置

2026-08-12 的第一次 supplemental closure 尝试已由用户终止：

- integration branch 停在 `codex/phase-0-supplemental-closure` / `70c65de5`；
- 约三小时内只产生审计、设计、测试设计、review 和 Execution Map 文档；
- 没有 production、test 或 Gate 非文档提交；
- 第一次 Design Review 为 `FAIL`；
- 所有该次 worktree 已删除，未提交设计修订已丢弃；
- 该 branch 没有合入 `main`，也不构成 Phase 0 receipt。

该 branch 可以作为问题线索读取，但其中的 audit/design/test/review 结论都不是 authority，不能整体 cherry-pick、
不能作为新 MAP 的 baseline，也不能让新 Agent 对其作形式化 disposition。需要使用的事实由当前代码或 accepted
文档就地核对；需要选择的目标按 §6 条件启动 Design task。

## 2. 共同 Phase Contract

Development/Proof 两条线共享以下合同。

### 2.1 必须闭合

1. 一份真实 `.skiff` fixture 经 production compiler 产生 canonical artifact；
2. artifact 经过 production-owned publication/store、deployment load/admission、image/cache、route/entry 和
   request boundary；
3. VM 实际执行 scalar/local-call fixture，并从 production response 返回确定结果 `3.0`；
4. harness 不直接构造 linked/verified/executable image、entry target 或 VM fiber；
5. route、entry、至少一次 VM dispatch、terminal 和 cleanup 来自 production-owned observation，而不是 harness
   写死；
6. corrupt artifact、wrong entry/deployment 和 unsupported request/capability 在真实边界 fail closed；
7. Gate 从 raw outcomes/events 生成 verdict，保存 durable evidence，并拒绝 dirty/stale/missing/zero/skip/tamper；
8. exact Phase 1 authority/verifier handoff 没有两套最终接口；
9. frozen candidate 由全新 Acceptance Agent 独立运行 canonical Gate 并给出 receipt。

### 2.2 非目标

本任务不：

- 实现 Phase 1 trusted scalar refactor 或删除整个 verifier；
- 修复 Phase 2–7 的 aggregate lifecycle、exception、Pending、HTTP stream、cross-owner 或 GC；
- 为 proof 新增 test-only execution path、fake seal、unchecked constructor 或第二 composition authority；
- 重做全仓架构审计或为每个 review finding 写详细迁移计划；
- 运行笼统 full repository verify；
- 因旧实现已合入 `main` 就降低 acceptance 标准。

若真实 VCP 只能通过新增公共 execution authority、改变用户可见语义或扩大 Phase 1 支持面才能建立，触发 §6
Design/用户决策，而不是由 Proof Agent 偷加 seam。

## 3. 两条执行线

```text
MAP0-R + Phase Contract preflight
  ├─ Development Line
  │    D0-O production observability / composition repair（仅按触发条件）
  │    D0-K urgent containment repair（仅按触发条件）
  │
  └─ Proof Line
       P0-V production-shaped VCP fixture/harness
       P0-G canonical Gate + durable evidence + self-tests
       P0-N negative/structural/regression scenarios

conditional C0-* Clarification / DEC0-* Design 只喂给受影响 task
  -> rolling integration + executable attempts
  -> frozen candidate
  -> fresh independent Acceptance
  -> phase-0-closure result
```

Proof Line 从第一批 task 就开始写 executable test/script code，不等待完整测试设计文档。Development Line 在现有
production surface 足以支持真实 proof 时可以为空；“没有 production change 必要”必须由成功 VCP 和 reverse
proof 证明，不能由调查报告宣称。

### 3.1 Proof Line 首批任务

`P0-V` 与 `P0-G` 在 MAP0-R v0 后立即并行 ready。

#### P0-V — production-shaped VCP

- 复用或缩小现有真实 `.skiff` fixture，但必须实际执行 scalar slot、local call、arithmetic/branch 和 return；
- 从最高层现有 production composition 入口进入，不直接调用内部 linker/image/target/VM constructors；
- 保存原始 process outcome 和 production events，不生成 `status: pass`；
- 先运行 expected-red，记录首个真实失败边界；
- 若只缺一个当前事实，提出一个 C0-* Clarification question，不自行扩成系统审计；
- 若缺只读 observation，向 D0-O 提交精确 event contract；
- 若缺 production seam 且新增 seam 会改变 authority，停止受影响 task并触发 Design。

#### P0-G — canonical Gate

- 注册唯一 Phase 0 selector/command；
- 验证 exact commit/tree、clean worktree、binary/harness identity；
- 聚合 focused、contract、VCP、negative、structural 和 required regression evidence；
- 将 raw logs/events、manifest、command receipt 写入 caller 指定的 durable output directory；
- manifest 由 aggregator 从 raw evidence 生成，harness 不得自报 verdict；
- Gate 自测至少拒绝 dirty、stale、missing、zero、skip、interruption 和 tampered evidence；
- 可先使用 deterministic fake raw inputs完成 checker expected-red/self-tests，不依赖 P0-V 才开始写代码。

#### P0-N — negative/structural scenarios

P0-N 可由独立 Proof Agent承担，也可在 P0-V/P0-G write set 不冲突时拆分：

1. corrupt artifact/index/target 在 production admission 失败；
2. wrong deployment/entry 不会 first-match 或 ambient fallback；
3. unsupported request/capability 不进入其它 executor；
4. harness 无 internal constructor/direct VM route；
5. raw evidence 缺失或被篡改时 Gate 非零退出；
6. Phase 0 原 Gate selector 作为 regression 或被明确替换，不能静默并存两套 canonical Gate。

### 3.2 Development Line 条件任务

#### D0-O — narrow production observability

只有 P0-V 给出“外部 outcome 无法证明 Phase Contract 中某个关键真实事实”的具体失败证据时启动。它只实现
必要的只读 typed event，不选择 route、不改变 execution、不生成 verdict。每个 event 必须有 production owner、
correlation identity 和 bounded payload。

#### D0-K — urgent containment

只有当前 production path 在 Phase 0 VCP/negative 中实际进入已知错误能力，且无法通过现有 accepted admission
稳定拒绝时启动。D0-K 只在唯一边界 fail closed；不实现该能力。若 containment 改变用户可见支持面而 canonical
文档没有答案，触发 Design/用户决策。

## 4. 角色和最小隔离

| Role | Owns | Must not do |
| --- | --- | --- |
| 主 Agent / Integrator | MAP0-R、派发、监控/接管、机械合流、freeze、handoff | 写最终 verdict、在 merge 时补语义 |
| Development Agent | D0-O/D0-K 等实际触发的 production repair | 修改 Gate 标准、验收自己 |
| VCP Proof Agent | fixture、production-shaped harness、raw outcome capture | 修改 production、直接构造 internals、生成 PASS |
| Gate Proof Agent | selector、aggregator、checker、durable evidence、自测 | 修改场景语义或 production execution |
| Acceptance Agent | frozen candidate 上运行 Gate、核对证据、receipt | 修改 candidate、修复、合流或更新状态 |

Clarification、Design 和 Design Reviewer不是预建角色，按 §6 的单个问题临时派发。设计者可以随后实现其决定；
它不能 review/accept 自己。VCP/Gate 可以由不同 Agent 并行；最终 Acceptance 必须是此前未写 candidate
production/test/Gate 的全新 Agent。

主 Agent 可以检查 handoff、write set、命令 receipt 和 join condition，但不能把这些机械检查伪装成独立验收。

## 5. MAP0-R、worktree 和进度控制

重新执行时必须新建并先单独提交：

```text
doc/implementation/bytecode-vm-convergence/tasks/phase-0-recovery-execution-map.md
```

不得继续或改写已中止的 `phase-0-closure-execution-map.md`。MAP0-R v0 只需记录：

- current `main` exact commit/tree 和 clean receipt；
- 本文 Phase Contract identity；
- P0-V/P0-G 首批 Agent、write set、worktree 和 join；
- conditional P0-N/D0-O/D0-K/C0/DEC0 的触发条件；
- `started_at`、短 `status_after`、partial handoff/takeover；
- first non-document commit 和 first executable attempt checkpoint；
- integration/candidate/evidence epoch。

### 5.1 时间与产物信号

本任务默认使用以下控制信号，实际时间写入 MAP0-R：

- Clarification 首次 checkpoint 不超过 20 分钟；
- 普通 Proof/Development task 首次 checkpoint 不超过 30 分钟；
- 目标在启动后 45 分钟内出现第一个非文档 commit；
- 目标在启动后 90 分钟内完成第一次 executable VCP/Gate attempt，失败也必须保存真实边界；
- 达到 checkpoint 仍只有扩写文档时，要求立即提交/交付部分证据并结束、拆分或 takeover；
- 单个 clarification/design 若预计需要长篇系统报告，必须先缩小问题，不能继续占据 critical path。

这些是调度/重排信号，不是用伪造空提交满足的 Gate。若真实 blocker 使目标不可达，主 Agent在 checkpoint
报告 exact blocker并重排，而不是静默等待数小时。

### 5.2 Worktree

- main checkout 始终停在 `main`；
- 建一个 Phase 0 recovery integration worktree；
- P0-V、P0-G 和并发 production writer 各用不重叠 write-lane worktree；
- Clarification 共享 exact detached baseline，通常不建分支；
- 同一 worktree 只有一个 write Agent；
- task 完整提交、handoff且 clean 后，同一 owner 的串行任务可复用 worktree；
- takeover 前停止旧 writer；未提交修改不是下游输入；
- acceptance 使用 frozen commit 的 detached clean gate worktree。

### 5.3 事件驱动调度

任一 Agent 完成、失败、中断或交付 commit 后，主 Agent立即核对 handoff、更新 MAP0-R、重算 ready frontier、
派发所有无依赖/写冲突的 task，并对满足 join 的 commit 机械合流。不等待一批 Agent 全部结束。

若一个 Agent超过 `status_after`：询问已完成产物、当前假设、blocker、可提交部分和剩余步骤；然后选择短
checkpoint、partial handoff、拆分、interrupt 或 takeover。Agent 数量服务于 critical path，不以填满并发槽为
目标。

## 6. Clarification、Design 和 Review 的触发

### 6.1 Clarification

只有“一个明确当前事实无法从 accepted input/当前 owner 的正常阅读中得到，且答案正在阻塞 task”时派发。
task 形式必须是：

```text
Question: 一个可回答的问题
Consumer: P0-V / P0-G / D0-O / ...
Baseline/read scope: exact
Output: 简短 answer + citations + unknowns
Forbidden: target API、迁移顺序、Phase owner、verdict
```

禁止预建 CAUD1–CAUD6，也禁止把 process/status、observability、containment 和 verifier migration 按子系统全部
重新审计。

### 6.2 Design

只有存在未决定目标选择，并且它影响多个 write owner、两条线共同合同、authority/ownership/failure 语义、
public/persistent boundary 或难以撤销时，才派一个 narrowly scoped Design task。

本任务可能触发的例子：

- 没有现有 production composition seam，是否新增及由谁拥有；
- production observation 会不会成为第二 execution authority；
- Phase 0 accepted verifier disposition 与 Phase 1 target interface 不能唯一对齐；
- containment 会改变 canonical 用户可见支持面。

Design receipt 只记录 decision、理由、被拒方案、Contract/API 影响、消费者、proof obligation 和未决项。高风险
共享决定按原则文档 §4.3 由独立 reviewer 审查；只阻塞其消费者，不阻塞 P0-G 等无关 ready task。

## 7. Rolling integration 和 Gate

每个 Development/Proof commit join 后运行受影响的最小可执行 preflight，并保存失败边界。推荐合流顺序由
MAP0-R 根据实际依赖决定，不预写固定 wave；通常 P0-G checker/self-tests 与 P0-V fixture/harness 可独立先合流，
D0-O 后合流并解锁真实 event assertions。

最终 canonical Gate 至少聚合：

| Evidence class | Required proof |
| --- | --- |
| production VCP | real source 到 response `3.0`，exact route/entry/VM/terminal/cleanup |
| admission negatives | corrupt/wrong/unsupported 在真实边界 fail closed |
| structural | harness 无 internal constructor、无 alternate executor/fake seal |
| evidence integrity | clean exact candidate、durable raw evidence、hash closure |
| Gate self-tests | missing/zero/skip/stale/dirty/tamper/interruption 全拒绝 |
| regression | 原 Phase 0 focused/request scalar 场景及明确 required selectors |

不要求笼统 full repository verify。required selectors 由 Proof Line在实现 Gate 时以最小可执行集滚动记录；既有
失败只有在 candidate 前已有 exact baseline evidence、owner、expiry 且证明不受本 task 影响时可 waiver。

## 8. Freeze 和独立验收

两条线 obligations 合流并完成 merged-state Gate preflight 后，freeze receipt 记录：

- exact commit/tree、clean status、worktree；
- Development/Proof commits 和 integration order；
- compiler/test/runtime/harness binary hashes；
- artifact/deployment/schema identities；
- canonical Gate command、required selectors 和 durable output location；
- conditional Clarification/Design 的问题、答案/receipt；
- Phase 1 accepted input 和仍 disabled/planned capability。

freeze 后任何 production/test/fixture/Gate/event/schema 修改都开始新 candidate/evidence epoch。

全新 Acceptance Agent 只接收本文、frozen candidate、canonical Gate command 和 durable evidence location。它：

1. 在 detached clean worktree运行完整 Gate；
2. 检查 harness 无内部旁路、manifest 来自 raw evidence；
3. 核对 exact route/entry/VM dispatch/terminal/cleanup；
4. 第一行给出 `PASS` 或 `FAIL`；
5. 记录 candidate/tree、commands、scenario counts、hashes、waivers 和 findings。

Acceptance 不修复。`FAIL` 返回对应 Development/Proof owner；修复后重新 freeze并开启新 acceptance task。

## 9. Acceptance checklist

- [ ] MAP0-R 从 current `main` 新建，未延续 aborted MAP/branch；
- [ ] P0-V 与 P0-G 从首批 frontier 并行启动并产生非文档代码；
- [ ] 所有 Clarification 都有具体 question/consumer，未形成全量审计；
- [ ] 只有满足条件的共享选择才产生 Design receipt；
- [ ] VCP 使用 real `.skiff` source 和 production composition，不直接构造 internals；
- [ ] external result 为 `3.0`，route/entry/VM/terminal/cleanup 来自 production-owned facts；
- [ ] corrupt/wrong/unsupported scenarios 在真实边界失败且无 fallback；
- [ ] Gate 而非 harness 生成 verdict；
- [ ] Gate 拒绝 dirty/stale/missing/zero/skip/tampered/interrupted evidence；
- [ ] canonical command 聚合全部 required evidence class；
- [ ] frozen candidate、binary/artifact identity 和 durable raw evidence hash 一致；
- [ ] Phase 1 handoff 对 verifier/image/entry 只有一个 target contract；
- [ ] 全新 Acceptance Agent 对同一 frozen candidate 给出 `PASS`；
- [ ] result/status 只在 valid receipt 后更新为 `accepted`。

## 10. Stop and user-decision conditions

主 Agent自主处理 Agent 分配、并发、worktree、task 拆分、重试、接管和合流。只有以下情况向用户提出最小
决策问题：

- canonical architecture 无法唯一决定用户可见语义或 semantic authority；
- production-shaped VCP 只能通过新增公共 execution authority；
- containment 会改变 canonical 支持面；
- verifier/image/entry 目标有两个不可兼容且权威资料无法排除的方案；
- required Gate 因外部权限/资源不可执行。

普通实现失败、expected-red、缺 observation 或 Agent超时不是用户决策；由主 Agent重排或触发条件 task。

## 11. Result and Phase 1 handoff

PASS 后新建：

```text
doc/implementation/bytecode-vm-convergence/results/phase-0-closure.md
```

至少记录：baseline、Phase Contract、candidate/merge/tree、MAP0-R final revision、实际 Agent/write lane、
Development/Proof commits、conditional questions/decisions、Gate command/selector matrix、durable evidence hashes、
Acceptance receipt、Phase 1 exact target contract、first ready Development/Proof tasks 和 disabled capability ledger。

原 `results/phase-0.md` 保留并标记 original acceptance withdrawn。只有 closure result 根据 valid receipt 合入
`main` 后，总计划才把 Phase 0 标为 `accepted` 并允许 Phase 1 production implementation。
