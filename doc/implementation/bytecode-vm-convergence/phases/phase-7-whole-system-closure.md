# Phase 7：whole-system closure, budget and final acceptance

> Status: planning ready; production/proof implementation blocked on Phase 6 accepted
>
> Planning baseline: `3f2e5ae3c6e62cba3e513c3941d31e5bd9cef4a0`
>
> Execution baseline: the exact clean Phase 6 accepted commit/tree and its canonical Gate assets, recorded before dispatch
>
> Semantic Closure: one exact candidate proves the accepted bytecode-only support surface, executable limits and whole-system composition

## 1. Activation condition

Phase 7 may start implementation only after Phase 6 is accepted and its result records the exact commit/tree, accepted
capability ledger and canonical workload specs. Read-only preparation may happen earlier, but no Phase 7 production, proof or
Gate lane may use this planning commit as a substitute for the Phase 6 receipt.

On activation, the integrator amends only the baseline fields and concrete write sets that cannot be known before Phase 6.
That mechanical activation does not reopen architecture review and does not authorize a wider support surface.

## 2. Inherited authority and failure envelope

Phase 7 inherits the
[`Phase 5 verifier hard cut`](./phase-5-typed-host-effects-resources-streams.md) and every later accepted tightening:

1. compiler source analysis/lowering is the sole authority for source type, effect, lifecycle, loan, placement and
   capability facts;
2. the artifact model owns the persistent schema/ISA and bounded structural validation;
3. the linker consumes exact compiler/artifact/registry facts, resolves exact references and returns one atomically
   constructed immutable `DeploymentExecutionImage`;
4. decode/index/CFG/stack/slot/call/resume consistency and statement mapping may be private bounded steps inside that
   constructor, but cannot form a separately callable verifier stage, facts bundle, seal or cache value;
5. linker, scheduler, request adapters and VM cannot infer missing source semantics from strings, context, nominal names,
   type shape, opcode shape or defaults;
6. malformed artifacts must produce a typed construction error or a checked safe request failure, with no panic/abort,
   out-of-bounds access, partial image publication, or pending/root/resource leak.

There is no production `bytecode-verifier` crate/stage/API, `VerificationSeal`, verifier-owned `Verified*` transport,
compatibility alias, selector or dual path. Phase 7 must reuse the Phase 5 structural reverse-search obligation rather than
recreate verifier-shaped proof infrastructure.

## 3. Scope and non-goals

Phase 7 closes:

- unified memory, fuel and hot-path limit gates over the mechanisms accepted by their owner Phases;
- only the bounded read-only observations needed to prove those gates;
- an exact supported/disabled capability inventory;
- whole-system execution of the HTTP, service, stream, task, interface, callback and Actor scenarios actually accepted by
  Phase 1 through Phase 6;
- final candidate/evidence integrity and independent acceptance.

Phase 7 does not first implement a language feature, boundary protocol, owner state machine, heap rule, lifecycle rule,
scheduler transition or Router execution behavior. If a whole-system or budget scenario exposes such a gap, the integrator
reopens the original owner Phase with an exact write set and keeps running every unaffected Phase 7 scenario. The Phase 7
proof/Gate owner and integrator must not patch production semantics to obtain PASS.

## 4. VCP-7 and Gate matrix

### 4.1 Exact-candidate regression composition

The Phase 7 Gate reruns the canonical Phase 1 through Phase 6 workload specs against the same exact Phase 7 candidate. It
imports/composes workload specifications at the runner level; it does not trust an earlier PASS receipt and does not invoke
nested Phase Gate processes. Nested Gates would create stale or split evidence epochs and can deadlock or bypass the single
Cargo lease.

Inherited stage sentinels remain the producer-to-consumer proof. Phase 7 does not duplicate them under new names or hand-build
artifact, linked facts, image, entry, fiber, owner token, response frame or execution result.

### 4.2 New closure obligations

| Gate | Required evidence |
| --- | --- |
| G1 inherited regression | every canonical Phase 1–6 workload executes on the exact candidate; no zero/skip/stale receipt |
| G2 schema/ISA identity | a real compiler-produced artifact records its candidate-owned schema/ISA and identity; structural validation and atomic image construction consume that same identity |
| G3 executable limits | accepted memory/fuel/hot-path limits terminate or reject bounded negative workloads and leave owner/resource counters balanced |
| G4 whole-system VCP | real supported HTTP/service/stream/task/Actor paths, plus interface/callback only if Phase 6 accepted them, traverse production compiler, artifact store, atomic image, Router/Runtime and final consumer |
| G5 structural fail-closed | no verifier residue, semantic reconstruction, fallback, second authority, hand-built proof seam or unsupported capability admission |
| G6 evidence integrity | exact commit/tree, clean worktree, command environment, all command receipts, tamper/zero/skip/stale checks, fmt/clippy and final fresh-status checks |

The schema and ISA are dynamic candidate identities, not literals copied from an earlier Phase document. G2 obtains them from
the canonical candidate constants and real artifact/image path, records them in evidence and checks exact equality. A schema
change opens a new evidence epoch; it does not add backward compatibility. An artifact fact change does not by itself require
an ISA bump when the opcode contract is unchanged.

### 4.3 Expected-red and no-fail-fast

If G2–G4 requires a new production observability or enforcement producer, the Proof Line first runs the affected real
scenario before that producer joins and records a nonzero, non-skip expected-red result. If Phase 6 already supplies every
required production fact and Phase 7 is closure-only, the proof instead uses controlled command failure, missing-receipt and
tamper self-tests to prove that the Gate becomes red and still executes every later reachable command; the real whole-system
baseline may be green.

The outer runner never stops on an ordinary workload failure. Every Cargo command uses `--no-fail-fast`, Cargo commands remain
serial under one exclusive lease, and all later reachable non-Cargo/Cargo commands receive receipts. Cargo serialization is
not permission to expose only one semantic failure per run.

## 5. Acceptance checklist

- [ ] Phase 6 is accepted; Phase 7 exact baseline/candidate commit and tree are recorded and clean.
- [ ] Phase 1–6 canonical workload specs rerun on this candidate; no historical PASS substitutes for execution.
- [ ] compiler-only source authority, atomic image construction and verifier hard-cut reverse search are green.
- [ ] schema/ISA/artifact/image identities come from and agree with the exact candidate; no old-schema compatibility path exists.
- [ ] memory/fuel/hot-path negative limits are bounded and leave pending/root/resource/heap owners balanced.
- [ ] the supported capability inventory agrees with the scenarios actually run; disabled lanes fail closed.
- [ ] the whole-system matrix uses real Router/Runtime composition and completes without fallback or hand-built execution facts.
- [ ] one red workload cannot truncate later commands; tamper, missing, zero, skip and stale evidence all fail closed.
- [ ] frozen implementation candidate receives a fresh read-only semantic review and a separate independent Acceptance PASS.

## 6. Stop and reopen conditions

Stop only the affected lane and amend the Execution Map before code changes when a scenario requires a new source fact,
artifact field, execution transition, owner state machine, Router behavior, production write-set extension or second
authority. Reopen the original semantic owner rather than assigning the fix to Phase 7 proof/Gate.

Documentation completeness, historical wording and unrelated architecture/reference drift are not Phase 7 blockers. A
real second authority, unsafe failure, false-green Gate, broken accepted invariant or unavailable exact composition seam is.
