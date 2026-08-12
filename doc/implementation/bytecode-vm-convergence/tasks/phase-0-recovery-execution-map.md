# MAP0-R：Phase 0 recovery rolling execution map

> Status: active; revision 4; P0-V expected-red integrated; P0-G/DEC0-S running
>
> Phase Contract: [`phase-0-supplemental-closure.md`](./phase-0-supplemental-closure.md)
>
> Baseline commit: `507779bedb009ec7789456995dd57df5e553739f`
>
> Baseline tree: `e32a16a1fb88f72457cb7fa7547e9fca950270fa`
>
> Integration branch: `codex/bcvm-p0-recovery`
>
> Integration worktree: `/Users/geek/workspace/skiff-bcvm-p0-recovery`

本 Map 从 current `main` 新建，不继续 aborted `codex/phase-0-supplemental-closure` 或其 MAP。主工作区保持在
`main`。Revision 0 在派发任何 Phase 0 recovery Agent 前单独提交。

## 1. Baseline receipt

在建立 integration worktree 前：

```text
$ git status --short --branch
## main...origin/main [ahead 185]

$ git status --porcelain=v1
<empty>
```

integration worktree 从上述 exact commit/tree 建立。aborted branch 只可作为线索，不是输入 receipt。

## 2. Shared Phase Contract

两条线共同证明：真实 `.skiff` fixture 经 production compiler、canonical artifact publication、production-owned
deployment load/admission/image/route/request 和 VM 执行返回 `3.0`；harness 不直接构造 internal image/entry/fiber；
raw production facts证明 exact route、entry、VM dispatch、terminal、cleanup；canonical Gate从 raw evidence 生成
verdict并拒绝 dirty/stale/missing/zero/skip/tamper/interruption。

Phase 0 不实现 Phase 1 VM refactor。若现有 production composition 或 observation 不足，先交 exact executable
failure；当前事实不明时条件派 Clarification，新增共享 authority 时条件派 Design。

## 3. Initial ready frontier

| Task | Line | State | Expected elapsed | `status_after` | Initial write set | Join condition |
| --- | --- | --- | --- | --- | --- | --- |
| P0-V | Proof | expected-red complete as `eb464566`; integrated as `5d72cfe3` | actual 12 min | completed before checkpoint | `runtime/request/tests/bytecode_vm_phase_0_vcp.rs`; scalar Phase 0 fixture files | canonical authoring/publication succeeds; raw evidence then stops at exact host-owned composition boundary; no direct constructors or verdict fields |
| P0-G | Proof | running as `/root/p0_gate`; started `2026-08-12T16:14:31Z` | 35–60 min | `2026-08-12T16:34:31Z` | `scripts/run-bytecode-vm-phase-0-gate.mjs`; new Phase 0 evidence/checker/self-test files; verify registry files only if required | non-document commit; durable raw-evidence/checker path; dirty/stale/missing/zero/skip/tamper/interruption self-tests; no harness-authored PASS |
| DEC0-S | Design | running as `/root/p0_seam_design`; started `2026-08-12T16:19:52Z` | 15–25 min | `2026-08-12T16:31:52Z` | `decisions/dec0-vcp-production-seam.md`; read-only production code | choose the existing production-owned VCP placement and minimum read-only observation contract without adding execution authority |

P0-V 与 P0-G 在本文件提交后并行启动。P0-G 初始阶段只运行 Node focused self-tests，不运行会触发 Cargo 的
canonical wrapper；P0-V 是首个且唯一获准运行 Cargo 的 Agent，避免共享 target 并发锁。后续 Cargo owner 由
Map revision 显式转移。

## 4. Conditional tasks

| Task family | Trigger | Output | Blocks |
| --- | --- | --- | --- |
| C0-* Clarification | 一个明确 current fact 无法由当前 owner 正常阅读回答 | short answer + exact citations + unknowns；默认不提交 | question consumer only |
| DEC0-* Design | 未决定选择跨多个 writer/两条线或改变 authority/public boundary | narrow decision receipt；必要时独立 focused review | decision consumers only |
| D0-O Development | P0-V executable evidence证明缺少 required production observation | narrow typed event commit | affected VCP assertions |
| D0-K Development | VCP/negative 实际进入已知错误能力且 admission 无法拒绝 | unique fail-closed containment commit | affected scenario |
| P0-N Proof | P0-V/P0-G 初始交付后可将 negative/structural write set精确拆开 | executable negative/structural cases | final Gate only |

不预建 CAUD1–CAUD6 或完整 Design/Test Design task。

## 5. Agent and worktree allocation

Revision 0 只预留首批 writer；实际 Agent ID 在 dispatch revision 中记录。

| Task | Branch | Worktree | Input | Cargo permission |
| --- | --- | --- | --- | --- |
| P0-V | `codex/bcvm-p0-vcp` | `/Users/geek/workspace/skiff-bcvm-p0-vcp` | revision-0 MAP commit | yes; one focused command at a time, output redirected if >30s |
| P0-G | `codex/bcvm-p0-gate` | `/Users/geek/workspace/skiff-bcvm-p0-gate` | revision-0 MAP commit | no during initial parallel frontier |

同一 worktree 一个 writer。Agent 使用 `fork_turns=none`，只接收 exact task contract。Acceptance Agent 尚未创建，
且必须与所有 candidate writer 独立。

## 6. Progress and takeover

- 任一 Agent 完成/失败/提交后立即更新 Map、合流满足合同的 commit并重算 ready frontier；
- P0-G 20 分钟、P0-V 25 分钟无可信产物时，要求报告已完成内容、当前假设、blocker、可提交部分、剩余步骤；
- 接近完成只给一个短 checkpoint；继续扩写分析而无代码时要求 partial commit/结束；
- 明显跑偏或异常时先 interrupt，再从最后可信 commit 建新 worktree派 takeover；
- 可并行派 read-only diagnostic Agent，但不允许第二 writer 同时接管同一 worktree；
- Phase 0 recovery 启动后 45 分钟无非文档 commit，或 90 分钟无 executable proof attempt，主 Agent必须记录
  blocker并重排，不以空提交或伪 proof 满足信号。

## 7. Integration and acceptance

integration owner 只机械合流满足 task contract 的 commits。每次相关 join 后运行最小 executable preflight；
不得在 merge 时添加 bypass、fallback、默认 owner 或 test-only seam。

Development/Proof obligations闭合后记录 exact freeze receipt，创建 detached clean gate worktree，并派此前未写
candidate production/test/Gate 的全新 Acceptance Agent运行完整 canonical Gate、核对 raw evidence、给出
candidate-specific `PASS`/`FAIL`。只有 `PASS` 才创建 `results/phase-0-closure.md` 并允许更新 Phase 0 状态。

## 8. Revision log

### Revision 0 — pre-dispatch

- recorded exact clean baseline and integration worktree;
- instantiated the shared Phase Contract without new semantic decisions;
- declared P0-V and P0-G as the first parallel executable frontier;
- reserved worktrees, write sets, expected elapsed time, short status checkpoints and Cargo ownership;
- created no Clarification, Design, Development repair or Acceptance Agent.

### Revision 1 — first executable frontier dispatch

- committed revision 0 as `9244f5b0` before any Agent existed;
- created `codex/bcvm-p0-vcp` and `codex/bcvm-p0-gate` worktrees directly under `/Users/geek/workspace` from that
  exact commit;
- dispatched `/root/p0_vcp` and `/root/p0_gate` with `fork_turns=none`, exact write sets and explicit elapsed/status
  expectations at `2026-08-12T16:14:31Z`;
- assigned initial Cargo ownership exclusively to P0-V; P0-G may run only focused Node self-tests;
- no Clarification, Design, production repair or Acceptance task is active.

### Revision 2 — production composition boundary discovered

- P0-V confirmed that the existing request-crate VCP cannot reach the production owner: `RuntimeHost::spawn_bytecode_request`
  and the crate-private `BytecodeDeploymentRegistry` own load/cache/admission/route, while `runtime/host` already depends on
  `runtime/request`; exposing/recreating that path in the request test would add a cycle or second composition authority;
- P0-V is producing a committable expected-red carrier that stops at that exact boundary rather than fabricating a seam;
- triggered one narrow Design task, DEC0-S, to decide host-internal VCP placement and the minimum correlated read-only
  observation facts; it does not reopen verifier/image/entry architecture or block P0-G;
- expected downstream tasks are a single host-owned VCP/observation Development lane and a resumed Proof assertion lane;
  exact write sets wait for the decision receipt and independent focused review.

### Revision 3 — DEC0-S dispatch

- created `codex/bcvm-p0-design` at exact input `dc7080fe` in `/Users/geek/workspace/skiff-bcvm-p0-design`;
- dispatched `/root/p0_seam_design` with `fork_turns=none`, a single decision question, one-file write set, no Cargo/test
  permission, 15–25 minute expectation and `2026-08-12T16:31:52Z` status checkpoint;
- P0-V remains owner of the expected-red carrier; P0-G remains independent and continues Node-only implementation.

### Revision 4 — P0-V expected-red join

- validated P0-V output `eb4645660f6a66baf1886916bdfd847ed10a6b80` / tree
  `815b3da402f1897a4a2d9dd7e39e9a59f1d7dba1` and integrated it as `5d72cfe3`;
- focused Cargo attempt exited `101` after canonical source compilation, authoring publication and exact release lookup, then
  intentionally stopped at host-owned `RuntimeHost::spawn_bytecode_request`; log SHA-256
  `16effdd9f9010f5efa64df3a2901f816c95236e4ce5ece586b241dc417e3d23d`;
- raw JSONL contained five non-verdict facts and SHA-256
  `968dc4f38a621c40c7914343ee4b4e5cd0ab4812f4da7f7f5b712edcb76cde09`;
- scalar typed-JSON fixture now exercises input, local call, arithmetic, comparison/branch and return without raw-HTTP
  aggregate/string/bytes workarounds;
- removed direct linker/verifier/image/entry/target/request execution and harness-authored PASS/count/bypass fields;
- newly exposed downstream prerequisite: typed-JSON `HttpBody` is currently materialized as bytes, so exact scalar entry
  admission requires a narrow production repair after DEC0-S; this does not block P0-G.
