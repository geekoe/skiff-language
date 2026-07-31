# A1/G1 tail-call finding-wave checkpoint

Status: `PRE_ACCEPTANCE_BLOCKED`

This checkpoint classifies the first frozen-candidate A1 and G1 failures and
defines the next bounded repair wave. It is an execution checkpoint, not a new
tail-call design and not an acceptance verdict.

## Authority, parents, and exact inputs

The direct parents are
[`parent-checkpoint.md`](./parent-checkpoint.md) and
[`f0-ready-to-freeze.md`](./f0-ready-to-freeze.md). Their authority trace ends
at:

1. [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md);
2. [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety).

The architecture and reference remain the only semantic authority. In
particular, this wave must not add a public control variant, persisted marker,
configuration, second trampoline, compatibility path, or new tail-call
eligibility rule.

The failed frozen candidate is:

- commit: `e08afdf4c4afa221e2bd749ce561716453198743`;
- tree: `f7f6857b9d78df8e37454b97ee38f363beb191f4`;
- task-before baseline: `c34a954bca3580533c153d5761e8805c423dbb09`;
- baseline tree: `8beb99c62fb2bf2f4fade9f41c855773c2e8a714`.

The candidate is no longer stable. A1 found production and evidence blockers,
and G1 stopped at phase 17. No evidence from that candidate can be promoted to
a completion verdict.

## Frozen A1 and G1 evidence

### A1 blockers

The independent A1 owner returned `FAIL` on the exact candidate/tree above.
Three blocker classes are established.

1. **The next tail entry loses the current `tailSite`.**

   `EvalContext::exec_program_return` creates the prepared frame with
   `call.site`; `Interpreter::exec_program_executable` accounts the transfer
   using that site, then destructures the prepared frame with `tail_site: _`.
   The next callable's `checkpoint_function_entry()` can fail after the
   transfer poll succeeded, but that failure bypasses call-site promotion.
   Existing instruction-limit evidence normally fails at transfer accounting
   and therefore masks this distinct entry-checkpoint path.

2. **The internal control result expanded the public Rust surface.**

   `runtime/eval/src/lib.rs` publicly exports `env`. The baseline public
   `env::Flow` did not contain `TailCall`; the candidate adds
   `Flow::TailCall(Box<PreparedTailCall>)` while keeping the payload
   `pub(crate)` and suppressing the mismatch with
   `#[allow(private_interfaces)]`. Making the payload public would deepen the
   violation rather than fix it.

3. **The authority-mandated dynamic matrix is incomplete.**

   Existing focused evidence covers the shared trampoline, direct/mutual and
   cross-module calls, generic/explicit self, ordered arguments, one record
   heap carrier, unequal plans, `ValueBlock`, `PackageDirect`, instruction
   accounting, non-tail depth, bounded diagnostics, 100,000 hops, the 1 MiB
   worker, and provider fresh depth. It does not dynamically close:

   - call-argument, catch, timeout, concurrent, DB transaction/lease, stream
     consumer/defer/stream-producing argument, service, Actor, and
     native/builtin negatives;
   - nominal, union, representation, and container carrier equivalence;
   - throw/catch/rethrow identity and correlation, including `traceId` and
     `errorId`;
   - the next-entry budget failure's exact current-tail-site attribution.

   Structure searches in the V2/F0 results do not substitute for runtime
   negatives explicitly required by the architecture.

The following A1 evidence remains useful but is not sufficient for acceptance:
there is one production trampoline, legacy and assembly share it, there is no
assembly-to-legacy fallback or persisted marker, and the existing positive,
pressure, non-tail, scheduler, compiler/linker, and real-source paths described
above are present.

### G1 classification

The unique G1 `pnpm verify` attempt passed phases 1 through 16, then failed in
phase 17:

```text
scripts/tests/command-execution-policy.test.mjs
actual lifecycle owners: expected 12, actual 11
```

The remaining 276 phases did not execute. The failure is a genuine
task-before-baseline defect, not a tail-call candidate regression and not an
environment classification:

- the complete `scripts` tree is the same object at baseline and candidate:
  `4491adf3e7495bf374116673586c7ba948135906`;
- the test, ledger, policy, scanner, discovery, and package blobs are
  bit-identical;
- the ledger has 11 unique owners: 9 `spawn` plus 2 `execFile`;
- the test still hard-codes 12 owners and 10 `spawn`;
- ancestor `37a18074f96f5e9710e0d9cfb8cc22aae4f8d32f`
  removed the obsolete `artifact-identity-cli` owner and its production file
  but did not reduce these two test expectations.

The only meaningful task-before comparison was run from clean exact worktrees,
using the gate's Node binary and the same shell environment:

```bash
/opt/homebrew/bin/node --test scripts/tests/command-execution-policy.test.mjs
```

| Input | Node | Result |
| --- | --- | --- |
| candidate `e08afdf4c4afa221e2bd749ce561716453198743` | `v25.9.0` | 9 pass, 1 fail; `11 !== 12` at line 23 |
| baseline `c34a954bca3580533c153d5761e8805c423dbb09` | `v25.9.0` | 9 pass, 1 fail; the same assertion and values |

The test writes only uniquely named OS temporary fixtures and cleans them in
`finally`; it does not write the repository, use a service, or access the
network. Both checkouts were tracked-clean and had the same production script
population. The detached baseline worktree was removed after the comparison.

## Authority to real production path

| Authority requirement | Real candidate entry and owner | Current evidence or blocker |
| --- | --- | --- |
| exact local tail eligibility | lowering `Return.value` -> linked `Call` -> `LinkedCallTarget::Executable`; `eval_context.rs::exec_program_return` | structural and positive runtime evidence exists |
| one internal prepared frame | `env.rs`, `program_execution/tail_call.rs::prepare_tail_call` | public `Flow` expansion must be removed |
| one iterative evaluator | `program_execution.rs::Interpreter::exec_program_executable` | unique trampoline exists |
| transfer and entry accounting at current edge | `eval_context.rs::account_tail_transfer`, `eval_context/checkpoint.rs::checkpoint_function_entry`, trampoline handoff | next-entry error discards `tail_site` |
| exact generic/self/return plan and heap carrier | `program_execution/tail_call.rs`, legacy and assembly invocation preparation/materialization | record/generic/self representatives exist; full carrier matrix is missing |
| lexical and target barriers | default-barrier `EvalContext::new`; timeout, concurrent, DB, stream owners; non-executable dispatch in `eval_program_call` | production shape looks fail-closed; required dynamic owner matrix is missing |
| bounded error semantics | call-site promotion, `RequestException`, local stack, catch/rethrow owners | bounded terminal throw exists; identity/correlation and entry-site cases are missing |
| real source consumer | `scripts/run-skiff-tests.mjs` -> compiler/File IR -> assembly/link -> shared evaluator | existing source fixture passes on the old candidate |

The critical masking order is:

```text
public/internal control and tail-entry handoff
  -> real barrier/target negatives
  -> carrier/error/entry-site matrix
  -> merged combined probe
  -> new preflight and freeze
  -> independent acceptance and one full gate
```

An R3 compile or control-conversion defect masks all later dynamic lanes. A
compiler/linker failure masks cross-module runtime cases. Within a barrier
case, ordinary-call/depth failure must be distinguished from cleanup or
arbitration failure before changing production code.

## Minimal repair DAG and owners

The counterfactual deletion review chooses the smallest closure:

- delete the public `TailCall` variant rather than publish its payload or add a
  public wrapper;
- carry the existing prepared frame and `tail_site` through a crate-private
  evaluator control seam; do not add a context side channel or second loop;
- test the real existing barrier and dispatch owners; do not add a general
  checker or a second barrier policy;
- correct stale tooling cardinalities; do not invent a replacement lifecycle
  owner or alter the production ledger.

### R3: internal control and tail-entry handoff

Ready after this checkpoint. R3 is the only production writer in this wave.

R3 owns:

- `runtime/eval/src/env.rs`;
- `runtime/eval/src/eval_context.rs`;
- `runtime/eval/src/eval_context/concurrent.rs`;
- `runtime/eval/src/flow_completion.rs`;
- `runtime/eval/src/program_db.rs`;
- `runtime/eval/src/program_execution.rs`;
- `runtime/eval/src/program_execution/tail_call.rs`;
- `runtime/eval/src/program_invocation.rs`;
- `runtime/eval/src/program_stream.rs`;
- only causally required mechanical `Flow` consumers in the same crate.

Completion requirements:

1. Restore the pre-candidate public `env::Flow` variant set. `TailCall` and its
   prepared payload must be representable only in a crate-private evaluator
   control type/seam. Remove the `private_interfaces` suppression; do not make
   the payload public.
2. Keep the single existing trampoline. Public or outer completion seams must
   receive only ordinary completion values or fail closed on an impossible
   internal escape; they must not become a second trampoline.
3. Retain the prepared frame's existing `tail_site` until the next callable
   entry checkpoints complete. Promote only transfer/entry preparation and
   entry-checkpoint failures with that site. Target-body throw/native source
   and the fixed real non-tail prefix must remain untouched; no duplicate
   call-site frame may be added.
4. Preserve the existing accounting/checkpoint positions, return-plan check,
   heap carrier, self/generic preparation, depth policy, barriers, and
   scheduler behavior.

R3 does not own matrix test files. Its focused proof owner may use:

```bash
RUSTFLAGS='-Dprivate_interfaces' cargo check -p skiff-runtime-eval -p runtime
cargo test -p skiff-runtime-eval tail_call_payload -- --nocapture
```

The leaf must replace the provisional test filter with the final internal
control-layout filter. It must also reverse-search the public API to prove that
`Flow::TailCall` and the lint suppression are absent.

### N1: dynamic barrier and excluded-target negatives

Blocked by integrated R3. N1 is test-only and may not repair production.

N1 exclusively owns:

- new
  `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_negatives.rs`
  and its one registration line in
  `runtime/eval/src/assembly_execution/ordinary/tests.rs`;
- `runtime/eval/src/program_execution/execution_scope_tests/evaluator_timeout.rs`;
- `runtime/eval/src/eval_context/concurrent/tests.rs`;
- `runtime/eval/src/program_db/tests/transaction.rs`;
- `runtime/eval/src/program_db/tests/lease.rs`;
- causally required helpers under `runtime/eval/src/program_db/tests/fixture/`;
- `runtime/eval/src/program_stream/current_scope_tests.rs`;
- `runtime/eval/src/program_stream/supervised_executable_tests.rs`.

The dynamic matrix must exercise the real owner or dispatch path:

- binary/wrapper retain their existing evidence; add call-argument and catch;
- timeout and concurrent preserve post-body deadline/arbitration ownership;
- transaction/lease preserve commit/abort/release and heap/lease cleanup;
- stream consumer cleanup, deferred producer, and stream-producing argument
  remain on their existing path;
- service, Actor, and native/builtin dispatch do not enter the local
  trampoline.

For exact local calls inside a barrier, a seeded depth boundary is the cheapest
diagnostic that proves ordinary-call execution. For service/Actor/native
targets, assert the real dispatch result/owner continuation and absence of an
internal control escape; a synthetic generic `Barrier` test alone is
insufficient.

Focused commands are the final test-name filters for timeout, concurrent, DB,
stream, and the assembly negative matrix. N1 must not run a selector or full
gate.

### E1: carrier, error, and entry-site evidence

Blocked by integrated R3 and parallel with N1. E1 is test-only.

E1 exclusively owns:

- `runtime/eval/src/program_execution/execution_scope_tests/tail_call_execution.rs`;
- `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_execution.rs`
  plus an optional nested test module below the same basename;
- `runtime/driver/eval/tests/program_execution/tail_call_execution.rs`.

It must dynamically close:

- a budget/control fixture where transfer accounting succeeds and the next
  function-entry checkpoint fails, using two distinct sites and asserting the
  current edge exactly once;
- nominal/catch identity, union branch, representation, and container element
  carrier parity between tail and corresponding ordinary materialization;
- a tail chain through throw/catch/rethrow that preserves payload, catch
  identity, correlation, `traceId`, `errorId`, the real non-tail prefix, and a
  bounded eliminated-tail stack.

The assembly file is already long. New carrier fixtures should live in a
nested module rather than extending it with another large flat helper set.
E1 may not change production or N1 files.

Focused commands are:

```bash
cargo test -p skiff-runtime-eval tail_call_entry_checkpoint -- --nocapture
cargo test -p skiff-runtime-eval assembly_tail_call_carrier -- --nocapture
cargo test -p runtime runtime_program_legacy_tail_call_error -- --nocapture
```

Final leaf contracts may refine the names, but each filter must be non-zero and
must remain owned by E1.

### T1: pre-existing tooling cardinality

Ready immediately and independent of R3/N1/E1. T1 owns only:

- `scripts/tests/command-execution-policy.test.mjs`.

It must change the stale title/count expectations from 12/10 to the canonical
ledger's 11/9 while retaining the unique-owner, two-`execFile`, no
`migration-pending`, and full production policy assertions. It must not modify
the ledger, scanner, discovery rules, or add a fake production owner.

Focused proof:

```bash
/opt/homebrew/bin/node --test scripts/tests/command-execution-policy.test.mjs
```

### DAG

```text
this checkpoint
├── R3 internal control + tail-entry handoff
│   ├── N1 dynamic barrier/target negatives
│   └── E1 carrier/error/entry-site evidence
└── T1 genuine pre-existing tooling cardinality
        \          |          /
         \---------+---------/
                   |
             I3 combined probe
                   |
        new F0 preflight and exact freeze
                   |
             independent A2 + G2
```

R3, N1, E1, and T1 have no overlapping files. N1 and E1 start only after the
R3 production control shape is integrated, so they test one stable internal
seam. There is no useful additional production or test node in this wave.

## I3 combined probe

After all four nodes are merged, the integration owner runs one cheap
merged-state probe. Leaf owners must record their final filter names so I3 can
replace the provisional filters below:

```bash
cargo check -p skiff-runtime-eval -p runtime
cargo test -p skiff-runtime-eval tail_call_entry_checkpoint -- --nocapture
cargo test -p skiff-runtime-eval tail_call_negative_matrix -- --nocapture
cargo test -p skiff-runtime-eval assembly_tail_call_carrier -- --nocapture
cargo test -p runtime runtime_program_legacy_tail_call_error -- --nocapture
/opt/homebrew/bin/node --test scripts/tests/command-execution-policy.test.mjs
```

I3 also performs these read-only structural checks:

- the public `env::Flow` has its baseline variant set;
- no `#[allow(private_interfaces)]` hides the internal control;
- exactly one production trampoline consumes the prepared tail frame;
- no marker, schema, config, environment variable, second loop, or tail-hop
  spawn was introduced.

I3 does not run `node scripts/verify.mjs --only runtime`, another selector, or
`pnpm verify`. Selector rebuild and the complete gate belong to the later
stable-candidate owners.

## Evidence invalidation

- Any R3 production change invalidates the first A1 verdict for the affected
  control, checkpoint, carrier, error, and trampoline surface. It also
  invalidates F0's old runtime selector and every G1 result as candidate
  evidence.
- N1/E1 additions do not disprove unchanged positive behavior, but they change
  the runtime test population and therefore invalidate the old runtime
  selector as completion evidence.
- T1 changes only tooling tests. It invalidates the old tooling phase and the
  old G1 run, but not tail-call runtime semantics.
- A compiler/linker, model, artifact, dependency, lockfile, generated output,
  execution-control, heap, exception, Actor, DB, stream, or scheduler change
  after I3 invalidates the corresponding focused lane and must be classified
  before freeze.
- A Node/Cargo/Rust toolchain, dependency installation, source root, shared
  target, process-exclusivity, capacity, or cache-identity change invalidates
  the relevant preflight environment facts.
- Any authority/reference or public ABI/wire/schema/config change stops this
  DAG. It cannot be absorbed into R3 or a test node.

The phase-1-through-16 G1 prefix is diagnostic history only. Because production
and the test plan will change, it must not be spliced into the next gate.

## Conditions for the next freeze

A new stability epoch may begin only when:

1. R3, N1, E1, and T1 are merged into one tracked-clean exact commit/tree, with
   no in-flight writer on the affected surfaces.
2. I3 passes on that exact merged state and the structural searches confirm
   the internal-only, single-trampoline minimal shape.
3. The authority matrix maps every required negative, carrier, error, budget,
   stack, scheduler, compiler/linker, and real-source criterion to a real
   executable test and a unique evidence owner.
4. The gate owner redoes non-verdict preflight for the new source, toolchain,
   dependencies, cache/capacity, selector plan, and process exclusivity.
5. The rebuilt runtime and real-source selector evidence required by that
   preflight passes on the new candidate.
6. The main owner freezes one exact commit/tree, then assigns one independent
   acceptance verdict and one unique full-gate owner. The new full gate runs
   `pnpm verify` once; it does not resume at old phase 17.

Until all six conditions hold, the integration state remains a
pre-acceptance candidate.
