# P5-F445H-I6K-R2 Host runtime assembly v2 fixture repair

## Traceability and DAG position

- Direct parent:
  `P5-F445H-I6K-independent-current-scope-acceptance-result.md`, blocking issue B2.
- The parent records its full evidence chain through the I6 refresh/consumer/combined
  results and phase plan to the sole architecture source
  `doc/architecture/package-service-contract-deployment.md`.
- Node: repair the deterministic Host full-crate gate blocker caused by the
  `active_runtime_assembly` test fixture still constructing a v1 assembly identity.
- Prerequisite: I6 independent acceptance at baseline
  `b5f991efc6b3dd191e8d73485aac03679fe6477c`
  (`48d82258e3698055916ac5541f3764fe1e8a0bc1`) classified B2 as fixture drift,
  while production strict identity is already v2.
- Unblocks: B2 closure and a later combined integration probe / new independent I6
  acceptance. It does not close Eval blocker B1 or accept I6 by itself.
- Shared interface state: `RUNTIME_ASSEMBLY_SCHEMA_VERSION`,
  `RUNTIME_ASSEMBLY_IDENTITY_PREFIX`, identity assignment, reference derivation,
  and Host admission all already require strict v2.

## Preflight facts and owners

- Real test entry:
  `runtime/host/tests/active_runtime_assembly.rs::
  rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent`.
- The fixture's committed assembly is built with
  `assign_runtime_assembly_identity` and referenced with `runtime_assembly_ref`, so
  it is already canonical v2.
- Its intentionally unknown reference alone is hard-coded as
  `skiff-runtime-assembly-v1:sha256:<64 lowercase hex>`. Strict lexical validation
  rejects it before the test reaches the intended content-resolver mismatch and
  `AssemblyActivationRejectReason::Resolve`.
- Baseline search finds no v1 generation in `runtime/host/src`; the only v1 string
  under the Host crate is this integration-test fixture.
- Production owner remains the existing artifact-model / artifact-identity / Host
  admission path. This task owns only the stale test fixture and its execution
  evidence.
- Upstream strict-validation failure masks the intended resolver-rejection and
  committed-generation preservation assertions in this one integration test.

## Write scope

- This task file and its result file.
- `runtime/host/tests/active_runtime_assembly.rs`: mechanically migrate the
  deliberately unknown but lexically valid reference from strict v1 to strict v2.
- No builder, golden, or caller beyond that file is expected from baseline
  preflight. A directly causal test-only mechanical fixture may be included if
  verification proves one was missed, and must be recorded in the result.

## Non-goals and stop conditions

- Do not add v1 compatibility, dual-read, fallback, or legacy parsing.
- Do not relax strict v2 identity validation or change any production semantics.
- Do not modify artifact identity/schema/public contracts or production builders.
- Do not touch the Eval B1 blocker, current-scope behavior, stable/live services,
  network, MongoDB, or `Cargo.lock`.
- Stop and report scope expansion if production still generates v1, if the repair
  needs a public identity design change, or if any production modification is
  required.

## Acceptance and evidence ownership

Risk is low: a test-only fixture migration under an already locked strict identity.
This development owner runs:

1. RED, before repair:
   `CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host --test active_runtime_assembly rejected_exact_ref_preserves_committed_generation_and_two_replicas_are_independent --locked -- --exact --nocapture`
2. GREEN, after repair: the same exact non-zero selector.
3. Host full crate gate exactly once after repair:
   `CARGO_NET_OFFLINE=true cargo test -p skiff-runtime-host --locked --no-fail-fast`
4. Locked wiring:
   `CARGO_NET_OFFLINE=true cargo check -p skiff-runtime-host --locked`
5. `cargo fmt --check`
6. `git diff --check`
7. Reverse search proving no v1 remains in Host production or this fixture.

The exact selector is the earliest risk probe. Completion requires its intended
Resolve rejection and state-preservation path to pass, the full Host gate to be
green with a reported count, locked check/format/diff to pass, and zero production
or lockfile writes.

## Candidate, invalidation, and integration

- Starting maturity: pre-acceptance repair candidate; the parent acceptance is
  FAIL and B2 is open.
- Completion advances only this branch to `B2_CLOSED`; integration and a later
  acceptance owner decide the shared candidate maturity and I6 verdict.
- Evidence is valid only for the final result commit/tree. Changes to the Host
  fixture, Host admission/activation, artifact identity/model dependencies,
  `Cargo.lock`, or test command/environment invalidate the affected evidence.
- Worktree:
  `/Users/geek/workspace/skiff-p5-f445h-i6k-r2-host-fixture`
- Branch: `codex/p5-f445h-i6k-r2-host-fixture`
- Integration owner: `/root/phase05_integration_steward`
- Submit fixture/task changes separately from the result evidence commit. Do not
  merge, delete this first-level worktree/branch, or push.
