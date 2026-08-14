# Phase 7：whole-system closure, budget and final acceptance

> Status: implementation-ready planning package; execution is blocked on Phase 6 accepted
>
> Planning baseline: `3f2e5ae3c6e62cba3e513c3941d31e5bd9cef4a0`
>
> Phase 6 planning handoff reviewed: `ee4805ef4ab785f288b734f845fae5912d33c29e` / `274c83d72ad2b93b449ef048d28dd05e1d0d4199`
>
> Execution baseline: the exact clean Phase 6 accepted closeout commit/tree and its canonical handoff, recorded by the activation amendment
>
> Semantic Closure: one exact candidate proves the accepted bytecode-only support surface, executable bounds and whole-system composition

This is the terminal convergence Phase. It owns closure proof, evidence and final acceptance; it does not own new language,
boundary or execution semantics. Execution write sets and rolling state are controlled by
[`MAP7`](../tasks/phase-7-execution-map.md), and the process remains controlled by the
[`runbook`](../runbook.md).

## 1. Activation contract and Phase 6 handoff

Phase 7 implementation starts only after Phase 6 is independently accepted, merged and clean. The Phase 6 planning handoff
named above fixes the API vocabulary but is not an accepted implementation baseline. Read-only preparation may
happen earlier, but this planning commit is never an execution baseline or PASS input. Before dispatch, the activation
amendment must resolve the following fields from the accepted Phase 6 result, frozen implementation candidate and final
closeout baseline, not from this document:

| Required handoff | Phase 7 use | Missing or ambiguous disposition |
| --- | --- | --- |
| frozen implementation candidate commit/tree and Phase 6 Acceptance evidence | provenance for the inherited implementation and prior verdict only; Phase 7 never substitutes the receipt for a rerun | activation blocked |
| accepted result/status-only closeout commit/tree, final `main` identity and clean status; proof that the frozen candidate is its ancestor and candidate→baseline changes only the Phase 6 result/status allowlist | parent of the Phase 7 integration line and first evidence epoch | activation blocked; any production/test/fixture/Gate/schema delta reopens Phase 6 |
| selector `bytecode-vm-phase-6-gate`; one cumulative `phase6WorkloadSpecs(root)` export; candidate-owned `phase6WorkloadProvenance(root)`; Gate spec/manifest/evidence schemas and contract tests | the sole inherited workload input; exact `sourcePhase/sourceId`, immediate `parentPhase/parentId` and ordered `originChain[{phase,id}]` cover Phase 1–6 without duplicate execution or ID-prefix inference | Phase 6 proof/Gate owner reopens |
| capability ledger with exact keys `service`, `task-function`, `task-Actor`, `interface-local`, `interface-remote`, `callback-same-runtime`, `callback-cross-runtime`, `Actor`, `DB`, `recoverable`, `request-GC` and `Actor-compaction` | chooses positive versus fail-closed rows in the matrix below; no variant is collapsed into a generic state and no enabled-but-unaccepted surface is allowed | Phase 6 owner reopens; Phase 7 cannot choose |
| accepted-lane pending/root/resource/child-heap/boundary-staging/memory-peak-release/Actor-arena observations | terminal balance checks for every applicable row | owning Phase reopens |
| memory ledger and executable limit; `request-GC`/`Actor-compaction` state and disabled/deferred disposition or accepted root receipt | mandatory memory row and conditional GC row | Phase 6 memory owner reopens if the mandatory limit is absent |
| `phase6BoundedWorkLedger(root)` with exact keys `p1-dispatch-fuel`, `p2-p3-cleanup-unwind`, `p4-wake-claim`, `p5-stream-pump-buffer` and `p6-materialization-root-walk`, each mapped to canonical transitive spec IDs | mandatory deterministic hot-path aggregation | the first missing feature owner reopens; Phase 7 cannot invent the bound |
| exact inherited workload inventory whose original `expectedTests` state is missing, `null` or an integer | candidate-specific input to the explicit P7G adapter/catalog; the original state remains auditable beside the effective count | Phase 6 proof/Gate owner reopens if the inventory is absent |
| compiler/artifact/image identity sources, observation schema identity and actual schema/ISA constants | dynamically bound Gate evidence | activation blocked; no literal is copied from an older plan |
| canonical compiler/runtime/router entry points used by accepted workloads | production-shaped composition, not a test-only seam | affected original owner reopens |

The activation amendment records the exact identifiers, exported symbol paths and exact Phase 7 write sets that cannot be
known now. This mechanical amendment does not reopen an architecture-document review and does not authorize a wider support
surface.

## 2. Inherited authority and verifier hard cut

Phase 7 inherits the
[`Phase 5 verifier hard cut`](./phase-5-typed-host-effects-resources-streams.md) and every later accepted tightening:

1. source analysis/compiler is the sole authority for source type, effect, lifecycle, loan, placement, boundary,
   materialization, transfer/drop and capability facts;
2. the artifact model owns the persistent schema/ISA and bounded structural validation;
3. the linker consumes exact compiler/artifact/registry facts, resolves exact references and returns one atomically
   constructed immutable `DeploymentExecutionImage`;
4. decode/index/CFG/stack/slot/call/resume consistency and statement mapping may be private bounded steps inside that
   constructor, but cannot form a separately callable verifier stage, facts bundle, seal or cache value;
5. linker, scheduler, request adapters, Router and VM cannot infer missing source semantics from strings, context, nominal
   names, type shape, opcode shape or defaults;
6. malformed artifacts produce a typed construction error or checked safe request failure, with no panic/abort,
   out-of-bounds access, partial image publication or pending/root/resource/heap leak.

There is no production `bytecode-verifier` crate/stage/API, `VerificationSeal`, verifier-owned `Verified*` transport,
compatibility alias, selector or dual path. Phase 7 reuses the Phase 5 structural reverse-search obligation and must not
recreate verifier-shaped proof infrastructure.

## 3. Owned scope, non-goals and old open items

Phase 7 owns only:

- proof carriers that compose already accepted production paths into whole-system scenarios;
- one canonical Phase 7 Gate, its selector, runner, receipts, checker and fail-closed self-tests;
- a candidate-derived supported/disabled capability inventory;
- only a bounded read-only observation when an otherwise executable obligation cannot be observed through an accepted port;
- parallel frozen-candidate review coordination, independent Acceptance evidence and terminal project closeout.

It does not first implement an opcode, source fact, artifact field, linker rule, scheduler transition, boundary codec,
owner/root state machine, HTTP/Router behavior, heap rule, GC policy or execution limit. In particular, the old open items
are decided before execution as follows:

| Item | Phase 7 disposition | Semantic owner if the inherited fact is absent or wrong |
| --- | --- | --- |
| raw fuel and frame/dispatch bound | mandatory inherited Gate; rerun the accepted Phase 1 exact-boundary, overflow and deep-call workloads | Phase 1 budget/VM owner |
| unified per-request memory limit | mandatory inherited Gate across every accepted Phase 6 owner/heap lane; Phase 7 only composes and observes | Phase 6 memory/owner kernel |
| request GC/compaction | conditional: rerun only if the Phase 6 ledger says `accepted`; otherwise prove the declared disabled/deferred disposition remains unreachable and do not enable it | Phase 6 memory/root owner |
| hot-path bounded work | mandatory deterministic work-bound aggregation: VM dispatch/fuel (P1), lifecycle cleanup (P2/3), wake/claim (P4), bounded stream pump/buffer (P5), materialization/root walk (P6) | the first Phase whose canonical workload exposes the unbounded transition |
| wall-clock performance, throughput and optimization | optional non-blocking observation only when an accepted owner already defines a stable threshold; not a Phase 7 release benchmark | original subsystem owner in a separately authorized task |
| observability | reuse the candidate observation schema; P7O may add one read-only, non-decision-changing port after a concrete proof gap | original fact owner plus P7O observation owner |
| DB/recoverable | positive rerun only when the Phase 6 ledger says `accepted`; otherwise mandatory compile/admission/runtime fail-closed proof | Phase 6 DB/recoverable owner |
| VM-14 umbrella claim | closed only by the mandatory fuel, memory and deterministic bounded-work rows above; it creates no new Phase 7 enforcement authority | row-specific Phase 1–6 owner |

“Hot path” here means deterministic bounded work and bounded queues/buffers, not a flaky elapsed-time assertion. Phase 7 may
report measurements, but it cannot turn an unowned number into a release criterion during execution.

The activation amendment must also resolve named residuals from earlier result handoffs instead of rediscovering them during
Acceptance:

| Earlier residual | Required pre-Phase-7 disposition |
| --- | --- |
| effectful `SetWritablePath` RHS | an accepted owner workload proves the required staging, or the source/admission route remains disabled; semantic repair returns to the Phase 2 lifecycle and Phase 5 effect owners |
| COW partial-allocation/OOM orphan chain | an accepted failure workload proves cleanup and Phase 6 memory charging, or Phase 2/6 reopens before activation |
| root uncaught-exception payload teardown | a canonical Phase 4 request/root receipt proves it; a generic Pending regression label is insufficient |
| literal-branch catch identity and generated slot/map/representation handlers | each is either accepted by the Phase that enabled it or structurally/runtime unreachable with a fail-closed proof |
| historical Actor/durable/DB/compaction intentions | they first belong to the matching Phase 6 lane; old synthetic live projection cannot satisfy Phase 7 |

## 4. Whole-system VCP and coverage matrix

### 4.1 Exact-candidate regression composition

The Phase 7 Gate imports exactly one cumulative `phase6WorkloadSpecs(root)` from the Phase 6 candidate and adds only Phase 7
scenario and control specs. It does not import each cumulative Phase list separately, invoke Phase 1–6 Gate processes, or
reuse their PASS receipts. Earlier receipts establish provenance; every canonical workload executes again against the same
Phase 7 candidate and evidence epoch.

The composer must preserve command, args, cwd, environment identity, test format, the original `expectedTests`
missing/`null`/integer state, semantic lanes and
the exact `phase6WorkloadProvenance(root)` record. It rejects duplicate IDs/executions, an empty source-Phase group, a
non-bijective or non-monotonic origin chain, missing provenance, a test-formatted
spec without a positive exact test count, and an inherited `cargo test` spec without effective `--no-fail-fast`. An explicit
Phase 7 adapter catalog may add a missing historical `expectedTests` or normalize an inherited `cargo test` argument, but it
must be a reviewed `(spec id -> original state, effective value/change)` table covered by contract tests and bound into every
affected receipt. A non-test spec records `testFormat = null` and no effective count. The catalog cannot invent a default,
erase the original state or mutate `cargo build`, `cargo fmt` or `cargo clippy` arguments.

Inherited stage sentinels remain the producer-to-consumer proofs. Phase 7 does not duplicate them under new names or
hand-build artifacts, linked facts, images, entries, fibers, owner tokens, response frames or execution results.

### 4.2 Executable matrix

Each row is required. “Ledger-selected” means the Gate derives positive versus fail-closed expectation from the exact Phase 6
handoff, records that choice and rejects an unrecognized/omitted state.

| ID / surface | Semantic / proof owner | Production entry | Required expectation | Machine evidence |
| --- | --- | --- | --- | --- |
| C01 inherited Phase 1–6 closure | P1–P6 / P7G | candidate `phase6WorkloadSpecs(root)` | every unique inherited workload executes once; no nested Gate, historical PASS substitution, zero/skip/stale result or missing Phase provenance | spec-catalog digest, per-Phase/lane coverage report, command receipts and exact counts |
| C02 compiler/artifact/image identity | compiler + artifact + atomic linker / P7P | real `.skiff` source → compiler publication → RuntimeHost admission | candidate schema/ISA/artifact/deployment/image identities agree; missing/swapped/damaged facts fail closed; no verifier or semantic reconstruction | dynamic identity record, S1–S4 inherited receipts, malformed companion, reverse-search receipt |
| C03 HTTP unary | P1 and P5 / P7P | real HTTP client → Router gateway/dispatcher → Runtime session → request/VM → final response | exact service/version route, deterministic status/headers/body, one terminal, balanced owners | raw client response, Router/runtime route identity, terminal and inventory receipt |
| C04 HTTP server-stream | P5 / P7P | real HTTP client → Router WS→HTTP writer → Runtime host stream → provider | headers precede ordered bounded chunks and one end; cancel/disconnect/error releases handle, buffer and pending owner | chunk timeline, backpressure/cancel companion, resource/buffer/pending zero receipt |
| C05 service child | P6 / P7P | compiled caller → exact provider build → flat child trampoline → caller response | ledger-selected; accepted means distinct owner/heap success + ordinary throw + actual Pending; disabled means unique boundary rejects | Phase 6 service spec receipts plus whole-system response and owner/root chain |
| C06 function task and Actor task | P6 / P7P | canonical `task-function` or `task-Actor` ingress → scheduler/host/TaskStore and, for Actor task, exact Actor activation/lease seam → completion or restart | the two capabilities are selected independently; accepted paths preserve exact target/build, recoverable payload/materialization, lease/fence and one terminal across late/duplicate/retry cases; restart subcases require accepted `recoverable`, while disabled variants fail closed; DB commit and TaskStore submission are never claimed atomic | per-capability task receipts, build/payload/lease/fence identity, conditional restart/terminal evidence and disabled negatives |
| C07 interface dispatch | P6 / P7P | compiled Local/Remote interface call → exact table/target/carrier → result/error | ledger-selected; accepted Local/Remote variants run only those stated in ledger, with exact method/materialization facts; all other variants reject | dispatch identity, return/error receipt, disabled-carrier negative |
| C08 callback | P6 / P7P | provider callback request → same-Runtime callback owner → caller resume/cancel | ledger-selected; accepted callback preserves lifetime/owner and terminal once; cross-Runtime remains disabled unless explicitly accepted | callback owner/resume receipt, cancel/late negative, disabled-route evidence |
| C09 Actor | P6 / P7P | exact Actor id/build → Router/runtime Actor arena and lease/fence path → result/Pending/destroy | ledger-selected; accepted matrix covers exact build coexistence, lease/fence/session ownership, Pending and stale/late/destroy outcomes; otherwise fail closed | Actor route/build/lease/fence receipts and arena/root/resource terminal inventory |
| C10 DB and recoverable value | P6 / P7P | canonical DB transaction and recoverable codec boundaries selected independently by ledger | each state is ledger-selected; accepted path proves schema/materialization/transaction or codec cleanup, while a disabled surface has compiler/admission/runtime rejection as its only outcome | DB/codec identity, transaction/roundtrip receipt or per-surface disabled negative; no generic live selector substitute |
| C11 cancel/deadline/error mapping | P3–P6 / P7P | throw/VmFailure, due deadline, client/session disconnect and losing completion through real request path | one winner and one external mapping; late/duplicate losers cannot publish; unwind/partial values/pending/resources clean exactly once | outcome identity, HTTP/error result, race timeline and terminal inventory |
| C12 lifecycle and resource inventory | P2–P6 / P7P | aggregate/exception/Pending/HTTP/stream/cross-owner terminals | copy/move/drop/unwind/materialization balances every owner/root/resource/buffer/heap counter; no double release or orphan | before/after inventory, owner-specific cleanup sequence and failure receipts |
| C13 fuel/frame bound | P1 / P7G | request-owned execution budget → VM dispatch/local-call loop | limit N permits N attempts and rejects N+1; overflow/deep call bounded; terminal and settlement exact | inherited raw-fuel/deep-call receipts with exact test counts |
| C14 memory and conditional GC | P6 / P7P | accepted per-request memory ledger under aggregate, Pending, stream and cross-owner pressure | hard memory limit covers heaps, frames, sidecars, pending owners, resources, host buffers and accepted child/Actor owners; failure balances all owners; if GC accepted, compaction only at legal quiescence with complete roots; otherwise no GC route | ledger charge/limit/peak/terminal receipt, pressure companion and accepted-GC root receipt or disabled/deferred proof |
| C15 deterministic hot-path bounds | P1–P6 / P7G | dispatch, lifecycle cleanup, unwind, wake/claim, stream pump and materialization/root-walk workloads | every accepted loop/queue/buffer has an inherited finite counter/limit; an exceeded bound terminates or rejects without leak | owner workload receipts and summarized limit inventory; wall-clock timing is informational only |
| C16 capability and observation ledger | P6 / P7G | candidate handoff + actual manifest observations | all 12 exact capability keys have one declared state consistent with rows C03–C10; ordinary capabilities are `accepted` or `disabled`, `request-GC`/`Actor-compaction` additionally retain their explicit disabled/deferred disposition, and no enabled-but-unaccepted surface exists; observation schema identity is candidate-derived and contains no verdict authority | ledger/schema digests, exact-key row reconciliation, unexpected/missing/state-drift negative |
| C17 hard-cut and damaged-artifact closure | P5 / P7G | workspace/production graph plus real admission boundary | zero verifier crate/API/seal/selector/alias/dual path; damaged artifacts cause typed construction/safe request failure, never panic/OOB/partial image/leak | reverse search, dependency/selector graph checks and behavioral damaged-artifact receipts |
| C18 Gate/evidence controls | P7G | Phase 7 runner/checker self-test fixture | early ordinary red does not truncate later reachable commands; missing/unexpected/zero/skip/stale/tampered/reordered/cross-epoch evidence, active-lease contention plus unsafe stale-lease recovery, and path escape/symlink/directory swap each fail | runner sequence receipts, lease/evidence-root safety probes and independent checker negatives |

The activation amendment maps every row to exact candidate spec IDs and fails if any accepted capability has no positive row
or any disabled capability has no fail-closed row. A row may cite multiple command receipts, but no command may silently
stand in for a different semantic surface.

## 5. Gate runner and evidence contract

### 5.1 Candidate and command execution

The direct runner command is
`node scripts/run-bytecode-vm-phase-7-gate.mjs --candidate <40hex> --tree <40hex> --output-dir <absent-absolute-dir>`.
The public selector command is
`node scripts/verify.mjs --only bytecode-vm-phase-7-gate --jobs 1` with the three equivalent pinned environment fields
defined by MAP7. The Gate accepts caller-supplied `--candidate`, `--tree` and an absolute canonical absent `--output-dir`
outside and not an ancestor of the candidate repository. Repo/output roots and the output parent must resolve canonically
without a symlink; the evidence root is created exclusively and its directory identity is frozen. The Gate rejects ambient string `GIT_*` variables
except `GIT_PAGER` and snapshots a sorted normalized string environment for every spec. It never selects its
own HEAD or reuses an evidence directory.

Candidate evidence is four HEAD/tree/status triplets: preflight, postflight, closure and fresh, with status including
untracked files. An identity mismatch or dirty preflight is a safety stop and produces a complete FAIL assessment with all
unexecuted specs marked missing. Ordinary command failure is not a safety stop: the outer runner continues through every
later reachable spec, the remaining candidate probes and evidence finalization. Signal interruption is recorded and missing
work remains FAIL.

Before the workload epoch, the owner verifies there is no Cargo/rustc process or active earlier-Phase lease and pauses every
other Cargo-capable agent until release. All Cargo commands then execute serially while the runner holds exactly one
`/tmp/skiff-bcvm-p7-r1-cargo.lockdir` lease for the complete workload epoch and sets
`CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target`; it never runs `cargo clean`. Every `cargo test` command has
effective `--no-fail-fast`. Other Cargo subcommands retain valid native arguments. The Phase-specific directory prevents a
second Phase 7 runner; it is not claimed to lock legacy Phase runners, so the exclusive-agent precondition is mandatory.
Commands expected to exceed 30 seconds run once with captured durable stdout/stderr; an external operator polls the same
process/log rather than restarting it.

### 5.2 Spec and receipt schema

Every spec has a unique ID/execution, owner Phase, coverage rows, semantic lanes, exact command/args/cwd, normalized allowlisted
environment identity, `testFormat`, original `expectedTests` state and, for test-formatted commands, a positive effective
`expectedTests`. It also declares dependencies
and produced/required binary or artifact identities where applicable. Candidate probes are specs too. The receipt schema
records both count fields plus sequence number, start/finish time, normalized PASS/FAIL/BLOCKED/INTERRUPTED outcome,
`blockedBy`, stdout/stderr path/bytes/SHA-256 and prior-receipt digest. A failed producer creates a BLOCKED receipt for its
dependent consumer rather than allowing that consumer to use a stale shared-target binary; unrelated later commands run.

The Gate uses a deterministic SHA-256 receipt chain rather than an undeployed signing key:

1. receipts are written exclusively in canonical spec order;
2. receipt `0` binds a fixed genesis string, candidate commit/tree, Gate schema and spec-catalog digest;
3. receipt `n` records the SHA-256 of the exact previous receipt bytes and its own stream digests;
4. the manifest records the ordered receipt path/digest list and final chain head;
5. the checker reconstructs the chain and a sorted allowed-path closure of every non-manifest evidence file as
   `(path, bytes, sha256)`; it rejects every unexpected regular file.

Missing or unexpected receipts/files, duplicate or reordered receipts, command/environment drift, a non-PASS outcome, invalid
TAP/Rust summary, exact-count mismatch, skip/todo/cancel/ignore, stale/dirty candidate, changed ledger/schema/ISA/fixture or
cross-epoch composition all make the verdict FAIL. The manifest is a derived assessment, never authority; the checker
re-derives candidate, coverage, commands, counts, failures, chain and file closure without trusting stored verdict fields.
The CLI prints the SHA-256 of the final manifest bytes as the bundle's external anchor; the Acceptance result records it, so
the manifest does not attempt to contain its own digest.

### 5.3 Dynamic production identities

The manifest records compiler/runtime/router binary identities, artifact identity, schema, ISA, deployment/image identity,
observation schema, capability-ledger digest and workload-spec catalog digest obtained from the exact candidate production
path. It checks exact equality across compiler publication, admission and image construction. No Phase 7 source pins a schema
or ISA literal copied from an earlier Phase. A candidate, schema, ISA, fixture, assertion, checker, observation or ledger
change begins a new evidence epoch.

Failed or interrupted evidence is immutable. The checker may re-read it, but command execution never resumes or fills files
in that directory; every rerun uses a new canonical absent output directory. A stale lease is removed only after verifying
that no owning Gate/Cargo/rustc process remains and recording the interrupted evidence path.

## 6. Expected-red and proof sensitivity

Phase 7 is closure-only by default. When it adds no production producer, it must not break a valid production baseline merely
to obtain expected-red. P7G instead uses controlled fixture command failure, missing receipt, reordered/hash-tampered receipt,
zero/skip summary and stale-candidate cases to prove nonzero FAIL, fail-closed checking and no ordinary-failure truncation.

If P7O or a reopened original owner adds a real production observation or enforcement producer, the affected real row must
first run before that producer joins and retain a nonzero/non-skip expected-red receipt. That red proves the genuine missing
boundary, while inherited unaffected rows continue and are receipted. Test-only verdict fields, fake images, fake Router
frames or hand-built owner state are forbidden.

## 7. Reopen, batch-fix and evidence-epoch state machine

A failing row is classified before any production edit:

- source fact/emission/admission or fuel: Phase 1/2/3/5/6 owner named by the row;
- lifecycle/unwind: Phase 2 or Phase 3 owner;
- scheduler/Pending/cancel/deadline: Phase 4 owner;
- HTTP/resource/stream: Phase 5 owner;
- cross-owner/capability/memory/GC/DB/recoverable: Phase 6 lane named by the handoff;
- false-green, coverage, runner, receipt or checker: P7P or P7G;
- one missing read-only observation: conditional P7O.

The state transition is always:

```text
frozen candidate Fn
  -> same-HEAD parallel review / Gate finding
  -> seal one deduplicated blocker ledger
  -> unfreeze
  -> MAP amendment and exact original-owner write sets
  -> fix all independent blockers as one parallel batch
  -> join + targeted checks + full merged preflight
  -> freeze Fn+1 (new commit/tree and evidence epoch)
  -> same-HEAD targeted/full recheck as classified
  -> only then detached Acceptance
```

All reviewers finish before the blocker ledger is sealed; an early finding does not turn review into serial
find-one/fix-one work. The integrator and P7G never patch production. A fix outside the sealed scope, a support-surface or
authority change, or an unexpected Gate/checker change requires a full fresh review cohort on the new freeze; a strictly
bounded fix receives parallel targeted rechecks for every affected finding/domain. Acceptance always reruns the complete
canonical Gate.

## 8. Risks, recovery and non-authorities

| Risk | Prevention / recovery |
| --- | --- |
| cumulative specs execute prior Phases repeatedly | import only Phase 6 cumulative export; assert unique IDs/executions and per-Phase provenance |
| one early red exposes only one issue per run | outer no-fail-fast runner, per-command receipts and a self-test that observes a later command/fresh probe |
| incompatible `--no-fail-fast` added to build/fmt/clippy | normalize/audit only `cargo test`; contract-test exact effective args |
| stale or mixed evidence falsely passes | caller-pinned candidate/tree, absent directory, receipt hash chain, dynamic identities and independent re-derivation |
| unsupported capability gets promoted by a live smoke | exact Phase 6 ledger selects positive/negative expectation; generic selectors never grant acceptance |
| flaky performance blocks closure | deterministic work/queue/buffer limits are mandatory; elapsed-time measurements remain non-authoritative |
| proof adds a second authority | proof consumes production facts only; semantic mismatch returns to original owner |
| reviewer reports arrive serially and trigger serial fixes | same-HEAD cohort completes first; integrator seals one blocker ledger and dispatches a batch |
| dirty worktree/stash is lost at closeout | exact Phase 7 inventory; commit or archive recoverable objects before exact-name removal; never wildcard-clean other Phases |

Architecture/reference prose completeness, historical wording and unrelated documentation drift are not Phase 7 blockers.
A real second authority, unsafe failure, false-green Gate, broken accepted invariant, missing mandatory limit or unavailable
exact production composition seam is a blocker and reopens the exact owner above.

## 9. Acceptance and terminal deliverables

Acceptance requires all of the following on one frozen candidate:

- [ ] Phase 6 frozen candidate and accepted closeout baseline are recorded separately; every exact capability key has its
      accepted/disabled disposition and request-GC/Actor-compaction retain any disabled/deferred disposition.
- [ ] the cumulative spec catalog has complete Phase 1–6 provenance, unique execution and exact positive test counts.
- [ ] C01–C18 execute with no zero/skip/stale/missing/tampered evidence; ledger-selected rows match the handoff.
- [ ] compiler-only source authority, atomic image construction and verifier hard-cut reverse search are green.
- [ ] schema/ISA/artifact/image/observation identities are dynamically read from and agree with the exact candidate.
- [ ] fuel, memory and deterministic hot-path bounds terminate/reject safely and balance all applicable inventories.
- [ ] whole-system HTTP, enabled capability and disabled-capability paths use real Router/Runtime composition without fallback.
- [ ] controlled red proves sensitivity and no ordinary failure truncation; evidence chain/checker negatives fail closed.
- [ ] fresh same-HEAD semantic and proof/Gate reviewers all finish; the sealed blocker ledger is empty after required rechecks.
- [ ] a different fresh owner runs the full Gate in a new detached clean worktree and independently checks raw evidence.
- [ ] the result records candidate commit/tree, Gate/manifest/chain digests, evidence location, capability/limit ledger and review/Acceptance identities.
- [ ] result/status-only closeout is safely merged and pushed to `main`; Phase 7 evidence is outside removable worktrees.
- [ ] every Phase 7 worktree, stash and active branch/ref is either cleanly removed or explicitly archived by exact identity; unrelated Phase state is untouched.
- [ ] project status is `closed/accepted`, all Phase 7 agents stop, and no Phase 8 or follow-on task is started.

The final in-repo deliverables are the activated Contract/MAP, canonical proof/Gate assets, `results/phase-7.md`, and the
README status update. The final result links machine-verifiable raw receipts and hashes; it does not replace them.
