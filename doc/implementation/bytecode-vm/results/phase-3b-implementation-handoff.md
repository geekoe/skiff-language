# Phase 3B implementation handoff: deployment linker and semantic verifier

Document role: implementation handoff

Phase status: planned

Code checkpoint: `adddbc72c31c9d0135fd7d21c3e847f98532ca1b`

Coordination baseline: `338b23cc592601ef95b21f50b5aafcdfcfc2a53e`

This document records an implementation checkpoint for the deployment linker and semantic verifier. It is
not a Phase 3B result, does not change the phase status to `candidate`, and does not claim that production
ingress executes bytecode.

The long-term authority remains the canonical
[`bytecode-vm.md`](../../../architecture/bytecode-vm.md). Phase status and evidence rules remain owned by the
[`phase execution contract`](../phases/README.md). The ordered prerequisites and acceptance target are:

- [`Phase 2 compiler emission`](../phases/phase-2-compiler-emission.md);
- [`Phase 3A exact-build deployment owner`](../phases/phase-3a-deployment-owner.md);
- [`Phase 3B linker and semantic verifier`](../phases/phase-3b-linker-verifier.md);
- [`core parallel execution design`](../design/core-parallel-execution.md).

## 1. Exact checkpoint

The implementation checkpoint is a linear ancestor of the coordination baseline:

```text
338b23cc  doc-only worker coordination update
  |
adddbc72  prove stream-read resume sites
  |
8ebfeee3  link package-global literal constants
  |
dec0e8b5  prove exact local tail calls
  |
4d7dbe39  prove exact local effects and no-pending
```

At handoff authoring time:

| Repository | Path | Branch | Baseline commit | Tree | State before this document |
| --- | --- | --- | --- | --- | --- |
| skiff | `/Users/geek/workspace/skiff` | `main` | `338b23cc592601ef95b21f50b5aafcdfcfc2a53e` | `511761764824af21278377c73836ef2fb9af78c7` | clean |

The code checkpoint tree is `b4bc4a1257a358b06684ec8e6e552b1e14da7d4e`. Commit `338b23cc` changes only
`doc/worker-crate-parallel.md`; it does not change the code evidence described below.

To inspect the complete implementation sequence in chronological order:

```bash
git log --oneline --reverse 3262d535^..adddbc72
```

## 2. Landed contract slices

### 2.1 Typed statement attribution

The statement-attribution epoch is connected across artifact, identity, compiler, loader, linked transport,
linker, verifier and VM schedule consumption.

Key checkpoints are:

| Commit | Contract |
| --- | --- |
| `3262d535` | typed statement-attribution authority, manifest identity and fingerprinted charge rules |
| `2c6da16d` | bytecode/package identity binding for the statement epoch |
| `16795049` | loader recomputation and exact package manifest binding |
| `92ac4ef0` | linker transport of typed statement events |
| `9da146c4` | P1 exact row binding and immutable verified statement schedule |
| `9b8855b9` / `7b56c2ad` | compiler source-owner inventory and MIR source-event plans |
| `b151808f` | VM consumption of the verified schedule without scanning raw statement rows |
| `a63c2f5b` | compiler attachment of statement execution authority |

The schedule owns rowless `FunctionEntry` charges and reclassifies, rather than duplicates, LocalCall,
TailHop and LoopCheck events. This does not mean that the VM implements the corresponding opcodes.

### 2.2 Exact local effects and NoPending

`97e6d64b` retains authenticated callable-effect and exact-call authorities through admission and control-flow
proof. `4d7dbe39` derives whole-image effects for the supported exact-local subset.

The proof:

- consumes P1 canonical analyzed effects and P3 exact call plans;
- checks the complete supported effect lattice rather than trusting linked summaries;
- rejects caller underclaims while allowing conservative over-approximation;
- derives `no_pending` only when the canonical pending-category set is empty;
- remains iterative and bounded for recursive and mutually recursive local graphs.

Unknown summaries, non-empty inout path effects and unsupported pending modes remain fail-closed.

### 2.3 Exact local tail-call proof

`dec0e8b5` adds a verifier proof for ordinary monomorphic `TailCallLocal`.

The proof requires:

- an exact local target and transitive pending contract;
- a stack containing exactly the target arguments, with no prefix residue;
- exact argument/result concrete classes and lifecycle plans;
- empty active-region and writable-loan state;
- authenticated cleanup plans for every live caller slot;
- terminal CFG shape and a single verified TailHop attribution event;
- inclusion of the tail edge in the exact-local effect fixed point.

This is a verifier certificate only. `runtime/vm` still rejects `TailCallLocal` as unsupported and does not
consume the internal tail-cleanup proof as execution authority.

### 2.4 Package-global literal constants in the linker

`8ebfeee3` links the first strict package-global constant subset.

Supported inputs are:

- named package roots backed by `LocalNode` constant rows;
- literal nodes only;
- canonical builtin `null`, `bool`, `number` and `string`, or an exact literal carrier;
- package-global type origins with no specialization;
- lifecycle plans independently recomputed as Ordinary plus SnapshotShare policy;
- package-local constant rows relocated to image-global constant indices;
- stack-map values that copy the authenticated linked constant type and plan.

The linker performs deployment-wide checked graph/table accounting before semantic validation, then copies
the exact constants, roots and literal nodes deterministically. Cross-package rebasing and aggregate limits
have focused tests.

This slice does not support arrays, records, representation or implementation nodes, package-symbol constant
rows, anonymous constants or general frozen graphs. It also does not make non-empty constants executable:
the verifier still seals only the empty constant heap, and the VM still lacks full `Const` value lifecycle and
constant-heap resolution.

### 2.5 StreamRead resume certificate

`adddbc72` adds the first non-empty verifier resume certificate, limited to the exact pair
`StreamNext + PendingMode::StreamRead`.

The certificate proves:

- joint P1 binding of artifact-local descriptor identity, linked global row, function, site, raw operand and
  typed instruction target;
- independent P2 derivation of item type `T` from a normalized affine `Stream<T>` endpoint;
- ready-path and resume-path stack/slot isomorphism, including non-empty stack prefixes;
- endpoint Live/Mutate behavior and exact item/result lifecycle plan;
- immediate ordinary successor shape;
- `RaiseAtCurrentSite`, `RaiseAtSite`, unwind behavior and the independently verified original source site;
- a canonical Stream pending category, preventing a false NoPending proof;
- an immutable, sealed `VerifiedResumeSites` view with no public constructor or mutable entry point.

The certificate covers a successful item and the error route. It explicitly does not authenticate natural
stream end. Other `ActualWithResume` modes, EmitStream, service/actor/interface/callback/host resume paths and
CallLocalInOut remain fail-closed at their existing earlier gates.

## 3. Validation evidence

The final non-Live integration command at the code checkpoint was:

```bash
cargo test \
  -p skiff-runtime-loader \
  -p skiff-runtime-linked-bytecode \
  -p skiff-runtime-linker \
  -p skiff-runtime-bytecode-verifier \
  -p skiff-runtime-vm \
  -p skiff-runtime-eval \
  -p skiff-runtime-host
```

It exited successfully with 1,352 tests passed and no failures:

| Package | Passed |
| --- | ---: |
| `skiff-runtime-loader` | 76 |
| `skiff-runtime-linked-bytecode` | 52 |
| `skiff-runtime-linker` | 131 |
| `skiff-runtime-bytecode-verifier` | 163 |
| `skiff-runtime-vm` | 28 |
| `skiff-runtime-eval` | 498 |
| `skiff-runtime-host` | 404 |

Local evidence log: `/tmp/p3-final-runtime-integration-test.log`.

Additional focused evidence:

| Command | Result | Notes |
| --- | --- | --- |
| `cargo test -p skiff-runtime-bytecode-verifier` | PASS | 157 unit + 6 doc tests at `adddbc72` |
| `cargo clippy -p skiff-runtime-bytecode-verifier --all-targets -- -D warnings` | PASS | no verifier lint failure |
| `cargo test -p skiff-runtime-linker` | PASS | 130 unit + 1 doc test at `8ebfeee3` |
| `cargo clippy -p skiff-runtime-linker --all-targets --no-deps -- -D warnings` | PASS | linker-owned targets only |
| `cargo clippy -p skiff-runtime-linker --all-targets -- -D warnings` | BLOCKED | existing dependency lints in `runtime/boundary`; linker was not reached |
| direct Rust formatting and `git diff --check` | PASS | final checkpoint |

These commands are useful focused evidence, not a Phase 3B verdict. This epoch did not run:

- the canonical `foundation`, `compiler`, `runtime`, `test-runner` and `checks` selector set;
- the mandatory isolated `router-live:agine` evidence;
- stable chat/host-tools closure;
- runtime/router rebuild, restart or binary SHA recording;
- the atomic image publication/cache acceptance matrix.

## 4. Fail-closed boundaries and non-claims

The following are deliberately still unavailable:

1. **Tail and ordinary call execution in the VM.** The verifier proof exists, but the current VM dispatcher
   still classifies CallLocal, TailCallLocal and Jump as unsupported.
2. **Non-empty frozen constant safety.** The linker can transport the literal subset, but
   `prove_and_build_empty_constant_heap` rejects non-empty authority with
   `ProofUnavailable(FrozenConstantSafety)`. VM constant resolution is also absent.
3. **Production StreamRead transport and execution.** The compiler does not emit the supported resume form;
   the production linker still rejects non-empty resume tables; the VM has no parked continuation or stream
   endpoint execution and `resume()` still rejects unexpected resumes.
4. **Natural stream end.** No canonical outcome contract exists in this slice. Do not infer one from the
   successful-item or error certificate.
5. **Other pending and target families.** CallLocalInOut, service, actor, interface, callback, host,
   backpressure and other ActualWithResume paths remain unsupported or fail-closed.
6. **The remainder of Phase 3B.** Generic monomorphization closure, exception regions, complete
   service/actor/interface target tables, callback/resource proofs, full frozen graph materialization and
   atomic image publication/cache evidence are not complete.

No verifier failure may fall back to tree execution, a permissive linker or runtime specialization. The
unsupported paths above must stay explicit until their independent authority and corruption tests land.

## 5. Recommended continuation DAG

### 5.1 Literal constant verifier heap

The next bounded constants slice should:

1. complete P1 constant/node binding with total source-row coverage and exact source-plan/literal-node body
   comparison, while retaining the existing exact total root coverage;
2. reuse the existing all-row P2 lifecycle proof and require every admitted literal row to classify as
   Ordinary/SnapshotShare;
3. construct a private sealed literal-only constant heap from authenticated hydration;
4. expose only immutable resolution by verified `ConstantIndex`;
5. keep arrays, records, representation/implementation nodes and package-symbol rows fail-closed;
6. add loader-backed body, type, plan, root, coverage and aggregate-limit corruptions;
7. only then add VM constant resolution in a separate execution slice.

### 5.2 Tail execution

VM tail execution must first extend the current verifier-private proof into a sealed VM-facing execution
authority containing the verified target and an exact slot/root/loan transfer-cleanup recipe. While the
caller frame is still intact, it must complete the TailHop and replacement FunctionEntry budget checks; any
failure leaves the caller untouched. It may then apply the recipe and commit the frame replacement as one
atomic transition while preserving bounded constant Rust-stack usage.

### 5.3 Stream execution pipeline

Proceed in dependency order:

1. freeze the natural-end contract;
2. link exact non-empty resume tables without weakening other resume modes;
3. emit the supported StreamNext descriptor from compiler-owned source/MIR facts;
4. implement VM parked continuation, affine endpoint ownership and item/error/end delivery;
5. prove statement events are neither replayed nor skipped across park/resume;
6. add a production-shaped source-to-VM test before claiming StreamRead execution.

### 5.4 Remaining Phase 3B acceptance

After the focused slices, return to the complete acceptance list in
[`phase-3b-linker-verifier.md`](../phases/phase-3b-linker-verifier.md). First complete the Phase 2 and Phase
3A prerequisites and close the full Phase 3B requirement ledger. Then run the canonical selectors and
isolated Live on one exact clean candidate. If they pass, create `results/phase-3b.md` with status
`candidate-pass` using the phase result template. After the candidate is merged, append the stable closure
receipt from the exact main merge commits and update the status to `complete`.

## 6. Working rules for the next owner

- Keep the main checkout on `main`.
- Use one writer per crate; read-only reviewers may run in parallel.
- Run Cargo commands serially because every checkout shares
  `/Users/geek/workspace/.skiff-cargo-target`; never run `cargo clean`.
- Redirect commands expected to exceed 30 seconds to `/tmp` and poll the existing process instead of
  restarting it.
- Stage exact paths and commit each validated slice promptly; preserve unrelated dirty work.
- The repository automated Rust file gate is currently 3,151 lines and Clippy's function
  `too_many_lines` threshold is 534. This tranche used the stricter working convention that new or newly
  responsibility-bearing files remain below 500 lines. The 500-line convention is a review rule, not a
  separate Clippy lint.
- Do not use a candidate stack map, resume row, target summary, effect summary or lifecycle plan as its own
  proof authority. Preserve the P1 hydration, P2 concrete-value and P3 control-flow separation.

## 7. Handoff verdict

Several substantial Phase 3B linker/verifier slices are merged and have focused plus seven-crate integration
evidence. Phase 3B as a whole remains `planned`: prerequisite accounting, the remaining verifier/linker
surface, atomic publication, canonical selector gates and Live/stable evidence are still open.
