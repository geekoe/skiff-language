# Phase 0 补充闭合任务

> Status: ready for execution
>
> Kind: Phase 0 recovery and acceptance closure; not Phase 1 implementation
>
> Historical implementation: `5592c694`
>
> Historical merge: `01f33c2f`（与 `5592c694` tree 相同）
>
> Authority: [`phase-0-architecture-reset.md`](../phases/phase-0-architecture-reset.md)、
> [`large-change-execution-principles.md`](../large-change-execution-principles.md)、
> [`bytecode-vm-architecture-review.md`](../../../architecture/bytecode-vm-architecture-review.md)

当前 Phase 0 已把一版审计、决策、VCP harness 和 selector 合流到 `main`，但原 acceptance 不满足 Phase 0
自己定义的独立验收、production-shaped VCP、candidate-specific evidence 和 Gate 聚合合同。本任务负责补齐
这些缺口，并在新的 frozen candidate 上产生一次有效的 Phase 0 acceptance。

本任务不是把全部工作重新交给一个 Agent。Phase 文档冻结角色、冲突约束、task DAG、proof obligation 和
停止条件；执行本任务的主 Agent 通过新的滚动 Execution Map 分配实际 Agent、worktree、并发和接管。

## 1. 当前状态和完成含义

执行本任务前，项目状态统一视为：

```text
Phase 0 implementation: integrated
Phase 0 acceptance: blocked
Phase 1 production implementation: not authorized
```

本任务完成必须同时表示：

1. 原 Phase 0 审计和架构决定经独立 Agent 重新核对并修订；
2. VCP 经过 production composition seam，而不是由测试手工拼装 linked/verified/image/target；
3. 关键事实来自 production 只读观测，PASS、bypass 和 fallback 不能由 harness 写死；
4. Gate 在 clean frozen candidate 上运行并保存 durable raw evidence、manifest 和哈希；
5. Gate 聚合本 Phase 声明的全部 required evidence，不靠人工拼接命令；
6. 当前危险的 enabled-unaccepted capability 有明确 containment 边界或 Phase 1 首要 prerequisite；
7. verifier disposition 已细化为可实施的删除/保留清单和 Phase 1 task；
8. 真正独立的 acceptance Agent 给出 candidate-specific `PASS`；
9. 所有状态文档只在上述 PASS 后恢复为 `accepted`。

## 2. 非目标

本任务不：

- 实现 Phase 1 的完整 trusted scalar refactor；
- 修复 Phase 2–7 所属的 lifecycle、exception、Pending、HTTP、stream、cross-owner 或 GC 语义；
- 以增加 verifier 规则代替 source/link/runtime authority 修复；
- 为了让 VCP 通过而增加 test-only execution API、fake seal 或 unchecked constructor；
- 把全仓所有既有失败都纳入本任务；
- 因为代码已合入 `main` 就降低 acceptance 合同。

若补齐 VCP 必须改变 production ownership、增加新的公开 execution authority 或扩大 Phase 1 support surface，
必须按 §10 停止并报告，不能藏在 validation infrastructure 中。

## 3. 必须关闭的问题

| ID | Blocker | Required closure |
| --- | --- | --- |
| P0C-01 | 原 MAP0 记录 single rolling owner，而 REV0-D/REV0-F 又声明独立 | 用可核对 Agent/task ID 和独立 receipt 重做 review/acceptance |
| P0C-02 | MAP0 只有事后汇总的单一 revision | 在首次派发前提交新的 MAP0-C，并保存每次派发、完成、接管和合流记录 |
| P0C-03 | harness 直接调用 linker/verifier/image/target constructors | 经审查选择真实 production composition seam；测试只使用该入口 |
| P0C-04 | VM marker、bypass/fallback 和 scenario PASS 由 harness 写死 | production typed events + Gate-owned verdict；缺观测即 FAIL |
| P0C-05 | manifest 位于临时目录并在 Gate 后删除，candidate 只检查 `HEAD` | clean/tree preflight + durable raw evidence/manifest + content hashes |
| P0C-06 | canonical command 只运行一个 integration test | 聚合 focused、contract、VCP、negative、structural 和 required regression evidence |
| P0C-07 | containment 以“Phase 1 尚未成为 production claim”为由后移 | 按当前 bytecode-only reachability 重做 ledger，危险路径变成首要 prerequisite |
| P0C-08 | DEC0 宣布删除 broad verifier，PLN1 exact interfaces 仍以 `verify`/`VerifiedVmEntry` 为目标 | 列出 exact retain/delete/migrate surface，并在 PLN1 中安排实现和 reverse gate |
| P0C-09 | result 没有 exact candidate commit/tree、durable manifest 和独立 receipt | 产生新的 closure result，保留原 result 作为无效历史证据 |
| P0C-10 | README、Phase 0 和 result 的状态互相矛盾 | acceptance 后由主 Agent 根据 receipt 机械统一状态 |

## 4. 角色合同

Phase 0 文档规定下列角色和冲突约束；实际 Agent ID 由 MAP0-C 记录。

| Role | Owns | Must not do |
| --- | --- | --- |
| 主 Agent | MAP0-C、派发、监控、接管、机械合流、freeze、根据 receipt 记录结果 | 编写 DEC/TST、实现 harness/gate、写 review、作 PASS/FAIL 判断 |
| Audit Agent | 一个 P0C read-only audit slice | 修改 candidate 或给最终 verdict |
| Architecture Agent | DEC0-C、containment、verifier disposition、PLN1-C | 实现或审查自己的设计 |
| Test Design Agent | TST0-C、event schema、scenario matrix、Gate contract | 实现 harness/gate 或作 acceptance |
| Design Review Agent | REV0-D-C | 修改设计、实现 HAR0 或参加 acceptance |
| VCP Development Agent | production-shaped fixture/harness | 修改 TST0-C、生成最终 PASS manifest |
| Observability Development Agent | 必要的 production 只读 typed events；条件角色 | 选择 execution route、生成 verdict |
| Gate Development Agent | selector、raw evidence aggregation、manifest/checker、Gate self-tests | 修改场景语义、实现 VCP execution path |
| Acceptance Agent | frozen candidate 上的 adversarial review、Gate run 和 receipt | 修改 candidate、修复失败、合流或更新状态 |

同一 Phase 内强制：

- 主 Agent 不兼任任何其它角色；
- Architecture Agent、Test Design Agent、Design Review Agent 必须是不同 Agent；
- VCP Development Agent 与 Gate Development Agent 必须不同；
- Design Review Agent 不得转为 Development Agent；
- Acceptance Agent 必须是此前未写 candidate、未写 Gate、未作 design review 的全新 Agent；
- 每个 Agent 从 exact commit 和 task contract 开始，不继承主 Agent 或作者的聊天结论；
- 资源不足只能降低并发或等待，不能合并上述冲突角色。

## 5. MAP0-C 和事件驱动调度

### 5.1 首个前置提交

派发任何 P0C task 前，主 Agent 必须创建并单独提交：

```text
doc/implementation/bytecode-vm-convergence/tasks/phase-0-closure-execution-map.md
```

MAP0-C v0 至少记录：

- exact baseline commit/tree 和 repo clean receipt；
- 所有 role 的实际 Agent/task ID；
- 当前 ready frontier；
- branch、worktree、write/read set 和输入 commit；
- `started_at`、`status_after`、是否可拆分和 takeover 方法；
- expected output、validation responsibility 和 join condition。

原 `phase-0-execution-map.md` 保留为历史，不覆盖或伪造其 revision history。

### 5.2 完成事件

主 Agent 不按整齐 wave 等待。任一 Agent 完成、失败、被中断或交付 commit 后，必须立即：

1. 检查交付是否满足 task contract；
2. 更新 MAP0-C 状态和 commit；
3. 重新计算 DAG ready frontier；
4. 派发所有没有依赖和 write-set 冲突的 ready task；
5. 对已满足 join condition 的提交进行机械合流；
6. 记录新的 candidate/evidence epoch 是否开始。

### 5.3 超时、跑偏和接管

达到 MAP0-C 的 `status_after` 且没有可信进展时，主 Agent 必须询问：已完成产物、当前假设、blocker、可提交
部分和剩余步骤。随后自主选择：

- 接近完成：给一个短 checkpoint 期限；
- 有设计 blocker：停止并退回 Architecture/Test Design owner；
- 长期无产物或明显跑偏：要求提交部分成果并结束；
- 原 Agent 异常：中断并保护 worktree；
- 可拆分：创建不重叠 write set 的多个 takeover task；
- 不可拆分：只保留一个新 write owner，其他 Agent 只读诊断。

同一 worktree 同时不得有两个写 Agent。可以同时派一个 write takeover 和多个 read-only diagnostic Agent。

### 5.4 命令策略

- 开发任务只运行 task contract 指定的 focused command；不得自行启动全仓测试；
- 预计超过 30 秒的命令把输出重定向到临时文件并可轮询；
- 当前不要求为所有 Cargo 命令设统一 hard timeout；MAP0-C 先记录观察时间和接管条件；
- required Gate command 无论耗时都必须产生完整 receipt；被中断等于未运行；
- baseline failure 只有在 Gate contract 事先列出 exact selector、baseline evidence、owner 和 expiry 时才可 waiver，
  不能用“既有失败”口头排除。

## 6. Task DAG 和最大安全并发

### 6.1 Read-only audit frontier

MAP0-C v0 建立后，以下六项全部 ready，应尽量同时派发：

| Task | Required role | Scope | Output |
| --- | --- | --- | --- |
| CAUD1 | Audit Agent | MAP chronology、role independence、status/result consistency | process gap evidence |
| CAUD2 | Audit Agent | production loader/image cache/route/admission/target composition seam | exact call graph and seam options |
| CAUD3 | Audit Agent | VM dispatch、entry、terminal、cleanup、fallback/bypass observability | typed event requirements and gaps |
| CAUD4 | Audit Agent | selector graph、candidate identity、manifest lifetime、Gate self-tests | Gate/evidence gap report |
| CAUD5 | Audit Agent | VM-01..VM-14 × all production ingress reachability/containment | exact capability ledger |
| CAUD6 | Audit Agent | verifier/seal/image/VM public surface and Phase 1 migration | exact retain/delete/migrate inventory |

每份报告必须引用 exact file/symbol/line/commit，区分当前错误语义、结构阻断和 fail-closed completion gap。

### 6.2 Design join

无需等待全部 audit 才开始草案：

- CAUD2/CAUD5/CAUD6 完成后，Architecture Agent 可开始 DEC0-C/PLN1-C；
- CAUD2/CAUD3/CAUD4 完成后，Test Design Agent 可开始 TST0-C；
- 两份设计只有在 CAUD1–CAUD6 全部纳入 disposition 后才能冻结。

Architecture Agent 和 Test Design Agent 使用不同文件/worktree；它们通过以下 handoff 对齐：

```text
Architecture -> support surface + authority map + allowed production seam
Test Design  -> proof obligations + typed event schema + Gate evidence matrix
```

若 Test Design 证明 production seam 不可观测或无法 fail closed，Architecture 必须修订，不能由开发 Agent
自行增加旁路。

设计产物：

- `CDEC0`：修订 DEC0 或建立带完整 disposition 的补充 decision record；
- `CTST0`：修订 Phase 1 Test Design Specification 和 Gate contract；
- `CPLN1`：修订 Phase 1 task DAG、K0 containment prerequisites 和 verifier migration lane。

三者冻结后，由全新 Design Review Agent 执行 `CREV0-D`。`FAIL` 时只退回对应 design owner；主 Agent 和
reviewer 不得修复。

### 6.3 Development frontier

只有 `CREV0-D: PASS` 后才能派发：

| Task | Required role | Readiness | Default write-set class |
| --- | --- | --- | --- |
| CHAR0-V | VCP Development Agent | production seam + raw event contract frozen | fixture and VCP harness |
| CHAR0-G | Gate Development Agent | scenario/evidence schema frozen | scripts selector, aggregator, checker, Gate tests |
| CHAR0-O | Observability Development Agent | CTST0 证明现有 production events 不足 | narrow production event sink only |

三项尽量并发。若 CHAR0-V 与 CHAR0-O 存在 API 依赖，先合流 event contract/interface 的完整原子 commit，再继续；
不允许 interface-only placeholder 进入 `main`。

开发 Agent 只交付 commit、focused evidence 和 blocker。它们不能写 `REV0-F`、result 或 status。

### 6.4 Integration and acceptance

主 Agent 在唯一 integration worktree 串行合流已接受的 development commit，不解决语义冲突。合流后：

1. 运行 non-acceptance preflight，确认 candidate 可供验收；
2. 冻结 exact commit/tree；
3. 创建 detached、clean gate worktree；
4. 派发全新 Acceptance Agent；
5. Acceptance Agent 独立阅读 Phase contract 和 candidate，不读取开发者总结；
6. Acceptance Agent 执行 Gate、检查 raw evidence，并给出 `PASS` 或 `FAIL` receipt；
7. `FAIL` 返回原 owner，任何修复开始新 candidate/evidence epoch；
8. 只有 `PASS` 才允许主 Agent 机械记录 result、更新状态并合入 `main`。

## 7. Architecture 和 containment closure

### 7.1 Verifier disposition

CDEC0 必须明确列出：

- `runtime/bytecode-verifier` crate 的最终 disposition；
- `VerificationSeal`、`SealedDeploymentFacts`、`VerifiedLinkedBytecodeImage`、`VerifiedVmEntry` 等 public type 的
  retain/delete/replace 结果；
- linker 输出和 immutable executable image 的 exact target type；
- image cache key、request admission、entry pin 和 VM input 的目标签名；
- 哪些 invariant 分别归 structural validator、linker 和 VM；
- Phase 1 的迁移 commit 顺序和 reverse-search Gate；
- 迁移期间允许的唯一临时状态，以及它为何不形成第二 authority。

如果决定删除 broad verifier，CPLN1 的 target interface 不得继续把 `verify` 或 `VerifiedVmEntry` 写成最终接口。
如果保留薄 verifier，必须逐条证明其职责不能由 structural validator、linker 或 VM owner 取代。

### 7.2 Containment

CAUD5/CDEC0 必须把 VM-01..VM-14 和所有 production ingress 一一映射到：

- 当前是否 reachable；
- 可能的数据破坏、阻塞、取消、identity 或 ownership 后果；
- 当前唯一 containment boundary；
- fail-closed proof；
- exact Phase owner；
- 是否是 Phase 1 启动前 prerequisite。

当前 reachable 且可能数据破坏、无限阻塞或无法取消的路径，必须：

1. 在本补充任务中 containment；或
2. 成为 CPLN1 中先于所有 scalar expansion 的 `K0` prerequisite，并拥有独立 Gate。

不能再使用“Phase 1 尚未成为 production claim”作为后移理由，因为当前 production 已是 bytecode-only。

## 8. VCP 和 Gate closure

### 8.1 Production-shaped VCP

VCP 从真实 `.skiff` fixture 开始，至少经过：

```text
production compiler
  -> canonical artifact publication/store
  -> production deployment load/admission
  -> production image cache/construction
  -> production route and exact entry selection
  -> production request entry
  -> VM scalar execution
  -> production response projection
```

测试不得直接构造或调用 `LinkedBytecodeCandidate`、`verify`、verified/executable image、`VerifiedVmEntry`、
`BytecodeRequestTarget` 或 VM fiber。当前 production seam 内部暂时调用 verifier 不构成测试旁路；关键是 harness
只调用 production composition API，Phase 1 可以在 seam 内替换 verifier。

若不存在这样的 seam，Architecture/Test Design 必须在开发前选择：使用现有更高层 process harness，或提出
最小 production composition seam。若后者改变 owner/authority，按 §10 停止并请求决定。

### 8.2 Typed observation

production 只读 event sink 至少观察：

- source fixture/content identity；
- artifact/package/deployment identity；
- load/admission/link/image outcome；
- exact route、deployment 和 selected entry；
- 至少一个实际 VM dispatch marker；
- response terminal；
- request cleanup/no leaked pending-resource-child owner；
- fallback/bypass event（若该路径存在）。

Harness 只保存 raw events 和 scenario process outcome，不写 `status: pass`、固定 dispatch 字符串或
`bypassCount: 0`。Gate 根据 raw events 和 expected scenario contract 生成 verdict；缺事件、重复 terminal、未知
route 或未识别 fallback 都是 FAIL。

### 8.3 Negative companions

同一 production composition 至少覆盖：

1. corrupt artifact/index/target 在 production admission 失败；
2. wrong deployment/entry/owner 不会选择首个 operation、其它 package 或 ambient artifact；
3. Phase 1 之外的 capability 在唯一 gate fail closed；
4. Gate 自身拒绝缺 manifest、zero scenario、skip、stale commit/tree、dirty candidate 和 tampered raw evidence。

### 8.4 Candidate and evidence

唯一 Gate command 必须：

- 在 detached clean worktree 运行；
- 验证 `HEAD` commit、tree hash 和 clean status；
- 记录 harness/test binary hash；
- 运行 CTST0 指定的 focused、producer-consumer contract、VCP、negative、structural 和 regression selectors；
- 由 Gate aggregator 生成 schema-validated manifest；
- 把 raw logs、raw events、manifest 和 command receipt 写到 caller 提供的 durable output directory；
- 输出每个 evidence class 的非零 count、pass/fail/skip 和 hash；
- 任一 required command 中断、skip、缺失或 candidate drift 时非零退出。

Evidence 不写回 frozen candidate。Acceptance Agent 把 evidence content hash、存储位置和 verdict 写入独立
acceptance receipt；PASS 后由主 Agent 在 post-candidate result commit 中归档 manifest/receipt 或其持久
content-addressed locator。

不强制本任务运行笼统的 full `scripts/verify`。CTST0 必须在开发前明确 required selector；未列为 required 的
全仓长测试不影响本 Phase，列为 required 的测试则不能因耗时或既有失败跳过。

## 9. Acceptance checklist

新的 Phase 0 acceptance 只有在以下条件全部成立时为 PASS：

- [ ] MAP0-C 在首次派发前单独提交，记录实际 Agent、worktree、时间、接管、commit 和 join；
- [ ] CAUD1–CAUD6 全部完成并被 CDEC0/CTST0/CPLN1 disposition；
- [ ] Architecture、Test Design、Design Review、Development、Acceptance 满足角色隔离；
- [ ] CREV0-D 对 exact design commit 给出 PASS；
- [ ] verifier retain/delete/migrate surface 和 Phase 1 task 已精确关闭；
- [ ] VM-01..VM-14 × production ingress containment ledger 完整；
- [ ] urgent containment 已完成或成为 CPLN1 最先执行且有 Gate 的 K0；
- [ ] VCP harness 不直接构造 linked/verified/image/target/VM internals；
- [ ] external result 和 exact route/entry/VM dispatch/terminal/cleanup 来自 production events；
- [ ] Gate 生成而非 harness 自报 verdict；
- [ ] Gate 拒绝 dirty、stale、missing、zero、skip 和 tampered evidence；
- [ ] unique Gate command 聚合全部 required evidence class；
- [ ] frozen candidate exact commit/tree 和 clean receipt 已记录；
- [ ] durable raw evidence、manifest、command receipt 和 hashes 已保存；
- [ ] 全新 Acceptance Agent 在同一 frozen candidate 上给出 PASS；
- [ ] closure result 记录所有 task commits、waiver、open capability 和 Phase 1 input；
- [ ] README、Phase 0、原 result 和新 result 的状态一致。

## 10. Stop and escalation conditions

主 Agent 可自主处理 Agent 分配、并发、任务拆分、worktree、重试、接管和合流顺序。只有以下情况停止并向
用户提交最小决策问题：

- canonical architecture 无法唯一决定 semantic authority；
- production-shaped VCP 只能通过新增公开 execution authority 才能实现；
- containment 会改变用户可见 support surface，而 canonical 文档没有决定；
- verifier 删除需要扩大 Phase 1 scope 或形成不可接受的双 authority；
- required Gate 因外部权限/资源而无法执行，而不是普通耗时；
- Design Review 连续拒绝同一架构选择且现有 authority 无法消解。

遇到这些条件时状态为 `blocked`。不能通过降低测试、手写 manifest、合并角色或先启动 Phase 1 绕过。

## 11. Result and handoff

PASS 后新建：

```text
doc/implementation/bytecode-vm-convergence/results/phase-0-closure.md
```

它至少记录：

- baseline、design、candidate、merge commit 和 tree hashes；
- MAP0-C final revision；
- 所有实际 Agent/task ID 和角色；
- audit/design/development commits；
- Gate canonical command 和 required selector matrix；
- durable raw evidence、manifest、receipt locator/hash；
- CREV0-D 和 Acceptance verdict；
- containment/K0 disposition；
- exact CPLN1 path 和 Phase 1 first ready frontier；
- waivers 和仍为 disabled/planned 的 capability。

原 `results/phase-0.md` 保留并标为 original acceptance withdrawn，不删除历史陈述。主 Agent 只能在 valid
acceptance receipt 后把总计划与 Phase 0 状态更新为 `accepted`；随后 Phase 1 才能建立 MAP1 和派发 production
task。
