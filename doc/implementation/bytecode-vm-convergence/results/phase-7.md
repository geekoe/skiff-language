# Phase 7 Result

> Status: accepted by canonical Phase 7 Gate PASS and independent Acceptance
>
> Candidate: `25dc7087fcf3a61e875c81bff19449b36f702ad6`
>
> Tree: `a7ee4c7dafbc22a017db0a22258f500f26ff0518`
>
> Gate verdict: PASS; 131/131 commands, 754/754 tests, checkerError null
>
> Evidence root: `/Users/geek/workspace/.skiff-bcvm-p7-e6-acceptance`
>
> Manifest SHA-256: `7509d37c65c728550f7b70d151f50211194c73f20dd80674cf0db0c71f939d4d`
>
> Acceptance: PASS by a fresh read-only Acceptance owner (P7A); no blockers

## 1. Delivered closure

- Whole-system proof carriers (`P7P`, MAP7 §3) walk the real chain
  `client HTTP -> Router gateway/dispatcher -> runtime WebSocket session ->
  RuntimeHost -> atomic image/scheduler/provider -> consumer -> terminal` for
  C02 identity, C03 unary, C04 server-stream, C05 service child, C06 task
  function/actor, C07 interface local/remote, C08 callback, C09 Actor, C10
  DB/recoverable, C11 throw/deadline/race, C12 lifecycle/terminal inventory and
  C14 memory ledger rows. No fake dispatcher frame, hand-built image or
  test-side projection is used.
- Gate, selector and evidence (`P7G`, MAP7 §3) implement
  `phase7ScenarioSpecs(root)` + exactly one imported `phase6WorkloadSpecs(root)`
  (re-IDed once with provenance), a deterministic receipt hash chain, a
  manifest/catalog/handoff layout, dynamic identity probes and the public leaf
  selector `bytecode-vm-phase-7-gate` (exclusive, slots 1, not in default
  verify).
- Phase 6 handoff and capability ledger are respected: ordinary capabilities
  accepted; `callback-cross-runtime`, `request-GC` and `Actor-compaction`
  remain disabled/deferred with fail-closed negative evidence (no package or
  release pointer published; typed compiler/admission rejection).
- Bounded-work keys are covered by inherited specs: `p1-dispatch-fuel`,
  `p2-p3-cleanup-unwind`, `p4-wake-claim`, `p5-stream-pump-buffer`,
  `p6-materialization-root-walk`.

## 2. Rolling fixes (P7R, sealed)

| Blocker | Root cause | Fix |
| --- | --- | --- |
| P7-BLK-01 | unary request switched to a stream response shape after child completion; Router's Unary pending rejected start/chunk with a deterministic 502 | `245806939` keep unary response shape |
| P7-BLK-02 | actor resurrection path held a non-reentrant instance guard across a second lock (self-deadlock) | `ca881a78e` drop guard before segment lease |
| P7-BLK-03 | string carriers are `HeapNode::Array` cells without an `array_slots` sidecar; JSON projection fell back to `[]` | `26ce0947c` project string carriers as JSON strings |
| P7R-2 batch | inherited specs blocked by stale/compile errors: scheduler test ports missing `child_stream_supervisors`; workspace clippy `too_many_lines` 543/534 + `SourceDependencyAnalysisInput::empty`; P7P fixture exposing owned image; Phase 5 stale public-stream sentinel + vcp harness runtime-binary gap; adapter test-count drift | `d8d3aaeaf`, `e68dae8a2`, `e772d60bc`, `1dc2d56fb`, `20f5f1167`, `08c82e34d`, `89a43d8a4` |

## 3. Review and acceptance

- P7S cohort on frozen HEAD: P7S-A (semantic), P7S-B (proof/Gate/evidence),
  P7S-C (whole-system capability). One blocker (P7P whole-system tests were not
  bound into the Gate matrix) fixed by `12cd9c6f6`; all remaining findings are
  advisory (recorded in MAP7 §16). Sealed blocker ledger empty; REVIEW_PASS.
- P7A first run (F2) FAIL: self-tests expectedTests 43 vs actual 44 (candidate
  defect, fixed by `89a43d8a4`) and an acceptance-worktree `ws` module gap
  (environment). Re-run on F3 PASS with fully independent chain/closure/
  identities/counts re-derivation.

## 4. Baseline residuals (accepted, no-fix)

- host lib full `phase_5_bytecode_http_*` stream-registry contamination and
  `runtime/request` `callback_provider_boundary...` ABI row fail only outside
  the Gate workload (host lib full / request crate full are not Gate specs) and
  are covered by whole-system positive evidence (C04/C08); recorded as accepted
  residuals per MAP7 §13.

## 5. Closeout

- Main merge/push identity and cleanup inventory: recorded in MAP7 §10 closeout
  steps by the integration owner after this result commit.

## 6. Post-acceptance fix and chat-smoke boundary

- Post-acceptance, user-authorized compiler extension `9497d7e51`
  (`feat(emit): support tagged-union field read for discriminator and branch
  access`) was merged to main. It enables `std.http.HttpSseEvent`-style
  tagged-union consumption (`.tag` discrimination + branch field reads) and
  was re-verified on main: Gate 7 PASS (evidence
  `/Users/geek/workspace/.skiff-bcvm-p7-e7`, 131/131 commands, 754/754 tests,
  manifest `786df311...`, epoch P7-E4).
- `e2e:chat-smoke` / `e2e:host-tools` are **not executable**: they require the
  `agine.ai/api` service, which cannot compile on the bytecode-only compiler
  because ordinary functions cannot return `string`/`Nullable<string>`
  (ValueShape admission; `bool`/`number`/record/stream return types compile).
  String exists only as a scalar carrier for the server-stream/throw/DB/
  actor-registry capability paths. Supporting string as an ordinary value
  shape is a cross-phase value-model extension outside Phase 7. Runtime/router
  verification therefore rests on the P7P whole-system proof tests and the
  Gate (P7-E4).