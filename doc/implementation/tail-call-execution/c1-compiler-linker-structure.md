# C1 compiler/linker structure evidence

Status: complete; pending integration

## Trace and DAG position

Direct parent:
[`parent-checkpoint.md`](./parent-checkpoint.md). Its authority chain continues
to the canonical tail-call architecture and runtime reference. This leaf only
owns C1 structural evidence and does not redefine those semantics.

- Node: C1 compiler/linker shape.
- Dependency: P0 is present at the assigned baseline.
- Unblocks: I1 after integration alongside R1 and S1.
- Shared production interfaces are already present:
  `StmtIr::Return -> ExprRefIr -> ExprIr::Call`, with local/publication call
  targets normalized by the assembly code linker to `ExecutableAddr`.
- Risk/acceptance: medium, evidence-only; the integrated batch receives the
  independent acceptance described by the parent checkpoint.

## Frozen input and ownership

- Repository: `/Users/geek/workspace/skiff`
- Baseline commit/tree:
  `c34a954bca3580533c153d5761e8805c423dbb09` /
  `8beb99c62fb2bf2f4fade9f41c855773c2e8a714`
- Branch/worktree:
  `codex/tco-c1-compiler-linker-structure` /
  `/Users/geek/workspace/skiff-tco-c1-compiler-linker-structure`
- Integration owner: `/root/tco_integrator`; this leaf must not merge or push.
- Unique selector owner: `node scripts/verify.mjs --only compiler`.

Expected write set:

- this leaf contract;
- compiler lowering structure tests under
  `compiler/lowering/src/source_file_lowering.rs` and its test submodules;
- one small linker test module under
  `runtime/linker/src/assembly/tests/`, plus its test-only registration.

R1 owns evaluator/runtime production. S1 owns provider scheduler depth. V1/V2/V3
own runtime, assembly-execution, and source-path behavioral evidence. This leaf
must not touch those surfaces.

## Executable completion contract

Compiler structure tests must follow each value-return's exact expression
reference, rather than merely finding a call somewhere in the executable.
They cover:

- direct self recursion;
- same-file mutual recursion;
- cross-module mutual recursion;
- a generic self call;
- an impl `self` call;
- a wrapped return whose selected expression is not a call;
- a call evaluated before a later return whose selected expression is not that
  call.

The linker test must start with unresolved local and publication executable
targets, run the canonical assembly relinker, and prove both become
`LinkedCallTarget::Executable` with the exact expected `ExecutableAddr`.

No production File IR, lowering, linker, artifact/schema, marker, checker,
registry, runtime, or tail-call evaluator change is allowed. No general checker
or persisted tail-call metadata may be introduced. If the existing structures
cannot express the matrix, this task stops as `TASK_NOT_EXECUTABLE`.

## Verification and maturity

Focused iteration may run the two new Rust test filters and formatting. Final
leaf evidence is:

```bash
node scripts/verify.mjs --only compiler
```

No full gate or runtime selector belongs to this leaf. The earliest risk probe
is the focused compiler lowering test plus the focused linker normalization
test. Passing C1 advances only an implementation checkpoint; it is not a
behavioral or frozen acceptance candidate.

Evidence is valid only for the submitted commit/tree. Changes to compiler
lowering, File IR call/return shapes, assembly file conversion, code linking,
address resolution, the fixtures used here, or compiler selector composition
invalidate the corresponding evidence.

## Read-only preflight result

Git-object inspection at the frozen baseline found the canonical owners and one
unambiguous test-only implementation path. Existing tests already provide
package lowering helpers and an assembly fixture/relink seam. The required
matrix can therefore close without production or public-contract changes.

## Result

Only test registrations, test modules, and this contract changed. The compiler
test follows each `Return.value` reference and covers direct self, same-file and
cross-module mutual recursion, generic recursion, impl-self recursion, plus
wrapped and staged non-tail negatives. The linker test injects unresolved local
and publication executable targets into canonical assembly files and proves
their exact normalized addresses.

Self-acceptance matrix:

| Task clause | Code evidence | Reverse-search evidence | Test |
| --- | --- | --- | --- |
| exact `Return.value` call selection | `source_file_lowering/tests/tail_call_structure.rs` | no production lowering or File IR changes | focused lowering test; compiler selector |
| wrapped/staged calls remain non-tail shapes | same compiler test module | only direct `Return.value` calls enter the positive helper | focused lowering test; compiler selector |
| exact local/publication assembly addresses | `assembly/tests/tail_call_structure.rs` | no production linker/address changes | focused linker test |
| no marker/checker/schema/registry mechanism | complete submitted diff | `tail` names occur only in test modules and this contract | diff/search plus compiler boundaries |

Evidence:

- `cargo fmt --all -- --check` — passed;
- `cargo test -p skiff-compiler-lowering tail_call_structure -- --nocapture`
  — 2 passed;
- `cargo test -p skiff-runtime-linker
  assembly_linker_normalizes_local_and_publication_targets_to_exact_addresses
  -- --nocapture` — 1 passed;
- `node scripts/verify.mjs --only compiler` — all selected phases passed.
