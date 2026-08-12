# MAP0-R：Phase 0 recovery rolling execution map

> Status: active; revision 11; D0-O, P0-G correction and host Proof running
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
| P0-G | Proof | first candidate `e17c3c35` rejected by receive review; correction running as `/root/p0_gate` | correction 20–30 min | 12 min after redispatch | `scripts/run-bytecode-vm-phase-0-gate.mjs`; new Phase 0 evidence/checker/self-test files; verify registry files only if required | exact production event/store facts; full image owner; exact process env; sole-mint structural proof; durable candidate-bound evidence; no harness-authored PASS |
| DEC0-S | Design | complete as `6ae2a8b1`; integrated as `49214d65` | actual 18 min | reported at overrun checkpoint | `decisions/dec0-vcp-production-seam.md`; read-only production code | host-internal VCP, five sole-mint observations, and D0-R/D0-M/D0-O/P0-V-H write sets decided |
| REV0-S | Review | `FAIL`, complete in 8 min; corrections recorded in rev8 | actual 8 min | on checkpoint | read-only DEC0-S and cited production code | found omitted event-owner/test files and confirmed D0-R sealed-fact expansion; no new authority found |
| D0-R | Development | takeover complete as `d12a9471`; integrated as `dd1399bc`; independent receive review PASS | actual 10 min takeover + focused validation | complete | DEC0-S host files plus approved narrow read-only accessors in `runtime/bytecode-verifier/src/verifier.rs` | route identity derives from pinned image owner; no request-time artifact reread; 7 focused host cases green |
| D0-M | Development | complete as `4a440017`; integrated as `e15bad88` | actual 11 min implementation + focused validation | complete before 18 min checkpoint | exact DEC0-S D0-M write set | seven typedJson cases and one rawHttp regression green; full file has one reproduced baseline failure |
| D0-O | Development | running as `/root/p0_observation`; started `2026-08-12T17:03:00Z` | 60–85 min | 20 min, then 45 min | corrected DEC0-S D0-O set plus approved existing driver owner `runtime/host/src/host/request_entry/resumable.rs` | dependency-neutral failure-isolated observer; five sole mint points; full deployment owner; request-local terminal/cleanup proof; focused tests |
| P0-V-H/P0-N | Proof | expected-red writer `/root/p0_host_proof`; started `2026-08-12T17:16:00Z` | 45–65 min | 15 min, then 35 min | two new host-internal test modules, module registration, removal of superseded request VCP; no production/Gate code | success and three negatives enter only `RuntimeHost::spawn_bytecode_request`, preserve actual wire/typed facts, and never mint verdict/internal execution objects |

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
| REV0-S | none; read-only | DEC0-S integration checkout | revision-5 MAP commit | no |
| D0-R | `codex/bcvm-p0-route-takeover` | `/Users/geek/workspace/skiff-bcvm-p0-route-takeover` | corrected revision-8 integration commit | yes after D0-M validation completed |
| D0-M | `codex/bcvm-p0-materialize` | `/Users/geek/workspace/skiff-bcvm-p0-materialize` | revision-5 MAP commit | no until D0-R releases the lease |
| D0-O | `codex/bcvm-p0-observation` | `/Users/geek/workspace/skiff-bcvm-p0-observation` | `dd1399bc` after D0-M/D0-R join | yes; one focused command after implementation, no workspace fmt |
| P0-V-H/P0-N | `codex/bcvm-p0-host-proof` | `/Users/geek/workspace/skiff-bcvm-p0-host-proof` | `f2a1cdc7`; expected-red until D0-O joins | no until rebased after D0-O; no production writes |

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

### Revision 5 — DEC0-S join and first Development frontier

- validated DEC0-S output `6ae2a8b1338cafd2f152ab4eea80748d9c5fd5c6` / tree
  `d43edbd2db1f286e45b17b37b6730553aaa0e687` and integrated it as `49214d65`;
- fixed the VCP at a host-internal `RuntimeHost::spawn_bytecode_request` test without a public or test-only execution seam;
- separated two executable prerequisites, D0-R pinned deployment/route identity and D0-M typedJson scalar materialization,
  from D0-O's five-event read-only observation propagation;
- opened D0-R and D0-M as disjoint Development writers and REV0-S as an independent read-only review; implementation may
  proceed in parallel, but a review rejection must be resolved before either Development commit joins;
- transferred the sole Cargo lease to D0-R. D0-M may write and commit a reviewable candidate but cannot run Cargo until the
  Map records lease transfer after D0-R's command completes.

### Revision 6 — production repair and review dispatch

- created D0-R and D0-M worktrees directly under `/Users/geek/workspace` at exact input `afc79c12`;
- dispatched `/root/p0_route_identity`, `/root/p0_typedjson_materialize`, and read-only `/root/p0_seam_review` with
  `fork_turns=none`, narrow contracts, explicit 8/18-minute status checkpoints and 12–45-minute elapsed expectations;
- D0-R exclusively owns Cargo; D0-M must stop at a committed untested candidate until the lease is transferred;
- P0-G remains an independent Node-only Proof writer and is producing its first detached checker/self-test commit.

### Revision 7 — D0-R sealed-fact accessor expansion

- D0-R stopped before edits after proving that host-only code cannot pin root service protocol, operation binding, ingress
  binding and verified adapter plan facts without an artifact-root reread or second sidecar authority;
- approved the DEC0-S conditional expansion to `runtime/bytecode-verifier/src/verifier.rs` for borrowed read-only accessors
  over the already consumed and admitted hydration/entry maps: service protocol identity, ordered operation IDs, ingress
  bindings and the verified gateway adapter plan;
- the expansion adds no constructor, mutation, alternate verification path or raw hydration accessor. D0-R remains the
  sole Cargo owner and must test these facts through the host route behavior rather than add verifier policy.

### Revision 8 — REV0-S rejection and correction

- REV0-S returned `FAIL` after 8 minutes because D0-O omitted `runtime/host/src/loader/bytecode_admission.rs`, the sole
  owner of its first two events, and the focused host route call sites that D0-R will change;
- durable review receipt is `reviews/rev0-s-vcp-production-seam-review.md`; DEC0-S now includes both files and explicitly
  forbids moving the event mint into non-owner `assembly_wire` code;
- the review's second blocker is satisfied by revision 7's narrow sealed-fact accessor expansion; D0-R is implementing it;
- D0-O is not ready until D0-R and D0-M join. Its cleanup proof must query the matching supervised request row, not infer
  correctness from a global active count.

### Revision 9 — D0-R containment/takeover and D0-M join

- watchdog inspection found the initial D0-R worktree dirty across 186 files after the writer ran `cargo fmt --all`; the
  Agent was interrupted immediately, the worktree was frozen, and no commit or non-target diff was accepted;
- the writer confirmed only three target-file patches were intentional and that no Cargo/rustfmt process remained. A clean
  `codex/bcvm-p0-route-takeover` worktree at `4f16bc35` was assigned to `/root/p0_route_takeover` with a 20–30 minute
  contract and 12-minute checkpoint; it may use the frozen target diff only as reviewed source material;
- validated D0-M output `4a440017ec45206d4873302a8cae7044aaed4da5` and integrated it as `e15bad88`;
- `cargo test --manifest-path runtime/request/Cargo.toml --test bytecode_request typed_json` passed 7/7, and exact
  `tests::raw_http_body_remains_heap_bytes` passed 1/1. The whole test file passed 12/13; its unrelated sleep fixture failed
  because the compile graph lacks canonical `skiff.run/std`, reproduced unchanged on the pre-D0-M integration candidate;
- sole Cargo ownership is now transferred to the D0-R takeover for its focused host test after the candidate is ready.

### Revision 10 — route join, observation dispatch and Gate receive rejection

- validated D0-R takeover output `d12a9471251a04a9c9d45d63434b72a999dab6f1` and integrated it as `dd1399bc`;
- the takeover changed exactly the three approved files. Four exact cases each passed once and the complete focused host module
  passed 7/7; an independent receive reviewer returned `PASS`, confirming cache hits do not reopen the store, route facts derive
  only from the admitted image, and the typed legacy sentinel cannot swallow other load failures;
- created `codex/bcvm-p0-observation` at exact input `dd1399bc` and dispatched `/root/p0_observation` with the corrected
  DEC0-S D0-O write set, one central writer, 60–85 minute expectation and 20/45-minute checkpoints;
- froze D0-O event serialization to full `ServiceDeploymentRef` image owners, typed route selector/role, canonical `Opcode`,
  shared root-request VM one-shot and exact request-local terminal/cleanup ownership;
- P0-G produced candidate `e17c3c35` and 11/11 green Node self-tests, but receive review rejected it before integration:
  bytecode identity was optional, image owner evidence was truncated, production-source strings and harness regexes could
  manufacture sole-mint evidence, and the recorded raw-output environment differed from the executed process;
- returned P0-G to its Proof owner for a bounded correction with new negative self-tests. No P0-G commit has joined the
  integration branch, and D0-O remains independent of the Gate verdict implementation.
- D0-O discovered that `drive_bytecode_request` currently destroys `BytecodeRequestExecution` inside the existing
  `resumable.rs` owner before the host finalizer can prove explicit pin release. Approved that one-file write-set expansion:
  the private driver returns a private driven result plus the optional concrete execution, callers explicitly drop it only
  after terminal handling, and no second run/resume loop or public execution seam may be introduced.

### Revision 11 — parallel host Proof frontier

- created `codex/bcvm-p0-host-proof` at exact input `f2a1cdc7` and dispatched `/root/p0_host_proof` with no Cargo lease;
- combined P0-V-H and P0-N under one Proof write owner because both add sibling host-internal modules and share the sole
  `request_entry.rs` registration point; their success/negative scenario logic remains separate;
- the Proof writer may read the in-progress typed observation and Gate schemas but cannot modify production or Gate code.
  It must first commit an expected-red carrier, then rebase after D0-O/Gate contracts join before executable validation;
- dispatched a bounded read-only `/root/p0_negative_clarify` to identify exact production setup and actual wire error facts
  for corrupt artifact, wrong entry and unsupported mode. It has no write set and only feeds P0-N;
- D0-O retains the sole Cargo lease. P0-G remains Node-only; host Proof does not run Cargo against an intentionally missing API.
