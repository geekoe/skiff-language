# V2 canonical assembly tail-call evidence

Status: ready for integration

## Trace and DAG position

Direct parent: [`parent-checkpoint.md`](./parent-checkpoint.md).

The parent checkpoint traces to
[`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md) as the
single architecture authority and to
[`../../reference/runtime.md`](../../reference/runtime.md) for user-visible semantics. Canonical
documents win any conflict.

V2 starts after the R1 shared evaluator checkpoint is integrated at
`bc4fea09685c6e92ed3fb5aa523ebc7cac6ba2df` /
`910bc17d1e752c81ae10a2105a77d89a113a292c`. It contributes the canonical
assembly evidence lane and unblocks I1 after V1 and V3 also integrate. The
candidate remains an implementation checkpoint; V2 does not freeze or accept
the overall tail-call candidate.

## Confirmed owner and entry

The real entry is
`Interpreter::execute_runtime_assembly_addr` over a linked
`RuntimeAssemblyEvalTarget`. Existing ordinary assembly test helpers create an
activation, request context, exact package image, request heap, and runtime
factory. R1 already owns recognition, prepared frames, return-plan comparison,
depth handling, and the only loop in `program_execution`; V2 only observes that
shared owner through canonical assembly.

The assembly linker normalizes local and publication executable references to
`LinkedCallTarget::Executable { ExecutableAddr }`. A lowering or linker failure
therefore masks runtime evidence and must not be repaired in this lane.

## Write scope and boundaries

V2 may write only:

- this leaf contract/result;
- new
  `runtime/eval/src/assembly_execution/ordinary/tests/tail_call_execution.rs`;
- the module registration in
  `runtime/eval/src/assembly_execution/ordinary/tests.rs`;
- a causally required test-only helper under the same ordinary assembly test
  owner.

V2 must not modify production code, the R1 shared evaluator, V1 driver tests,
V3 source fixtures, S1 scheduler evidence, compiler/lowering, linker, artifact
schema, selectors, manifests, or configuration. It must not introduce a second
trampoline, a legacy projection fallback, a test-only production bypass, or a
persisted tail-call marker.

If executable evidence exposes an R1 production defect, V2 stops with the exact
fixture, filter, error, and masked criteria. It does not repair R1.

## Completion matrix

The smallest sufficient matrix uses linked File IR fixtures and the canonical
assembly entry:

- direct recursion and same-file mutual recursion exceed the ordinary depth
  fuse and return a finite result;
- a cross-module publication cycle is linked to exact package/file executable
  addresses and executes beyond the fuse;
- generic substitution and static impl self recursion retain their parameter,
  return, and self carriers;
- a return inside a nested branch propagates to the same shared trampoline;
- call arguments are evaluated left-to-right exactly once before frame
  replacement;
- canonical-equivalent return plans preserve one shared-heap carrier and
  materialize the terminal value once;
- provably unequal return plans use an ordinary call and therefore hit the
  seeded non-tail depth fuse;
- one executable lexical-wrapper negative and one excluded-target negative
  retain ordinary behavior. Structure assertions/search cover catch, timeout,
  concurrent, DB, stream-deferred cleanup, service, Actor, native/builtin, and
  other non-`Executable` targets without creating a large fixture per owner.

The tests may group compatible criteria into table-driven fixtures, but a
passing result must still identify which case proved each criterion. Existing
C1 compiler/linker structure evidence remains the owner for source lowering
shape; V2 only adds the runtime-side exact-address assertion needed by its
cross-module executable case.

## Focused evidence and ownership

V2 owns only non-overlapping Cargo filters and formatting:

```bash
cargo test -p skiff-runtime-eval assembly_tail_call_
cargo fmt --check -- runtime/eval/src/assembly_execution/ordinary/tests.rs \
  runtime/eval/src/assembly_execution/ordinary/tests/tail_call_execution.rs
```

If Cargo does not support path-limited formatting in the installed toolchain,
run `cargo fmt` and verify that the resulting diff remains inside the declared
Rust write set. Do not run the runtime selector, compiler selector, Skiff
source suite, I1 combined probe, or full gate.

Evidence is valid only for the implementation/result commit descended from the
baseline above. Changes to R1 evaluator production, the assembly linker or
projection, artifact model, ordinary assembly test runtime, Cargo dependencies,
or execution environment invalidate the relevant results.

## Worktree and handoff

Repository: `/Users/geek/workspace/skiff`

Worktree: `/Users/geek/workspace/skiff-tco-v2-assembly-matrix`

Branch: `codex/tco-v2-assembly-matrix`

Integration target: `/root/tco_integrator` on `codex/tco-integration`.

After focused validation, V2 records the final matrix and exact commands in this
file, commits all scoped changes, and directly hands the branch, worktree,
commit/tree, actual write set, and evidence to the integration Agent. It does
not merge the shared branch, run a full gate, push, or clean its own一级
worktree.

## Result

The canonical assembly fixture uses one real `RuntimeAssembly`, package
identity assignment, assembly linker, activation, request context, heap, and
`execute_runtime_assembly_addr` entry. Depth-sensitive rows seed the context at
31: the entry consumes depth 32, so a shared tail transfer succeeds while any
ordinary nested program call fails with the exact
`programCallDepth/current=32/requestedDelta=1` payload.

| Criterion | Executable evidence |
| --- | --- |
| direct + nested branch | 96-hop direct recursion succeeds at the fuse and returns `0` |
| same-file mutual | two exact executable indices alternate for 96 hops at the fuse |
| cross-module mutual | publication edges link to exact package file indices in both directions and execute for 96 hops |
| generic | `T=string` is closed by the entry and retained through 96 self transfers |
| impl self | explicit-self-first impl recursion retains `"receiver"` through 96 transfers |
| argument order/once | two state-transition helpers require `start -> first -> second`; reordering or repetition fails |
| equal plan/carrier | the terminal record returns the identical request-heap handle and final mutated field |
| unequal plan | `Json` caller / `string` callee succeeds normally with the original value, but at the fuse takes the ordinary depth-checked path |
| lexical negative | `Return(ValueBlock(...Call...))` retains its continuation and hits the ordinary depth fuse |
| target negative | a real linked `PackageDirect` tail-position call remains non-`Executable` and hits the ordinary depth fuse |

The focused dynamic negatives are deliberately representative rather than a
large lifecycle fixture per owner. The frozen R1 source facts complete their
classification:

- `EvalContext::exec_program_return` requires all three gates:
  transparent context, `LinkedExprIr::Call`, and
  `LinkedCallTarget::Executable`; catch/binary/wrapper/call-argument results
  fail the expression gate, while service, Actor, native/builtin, interface,
  const-receiver, and package-direct calls fail the target gate.
- timeout, concurrent, DB, stream, and invocation owners either enter through
  barrier `EvalContext::new` / `Interpreter::exec_program_block` or reject an
  unexpected `Flow::TailCall`; they do not form a second trampoline.
- `tail_call_has_stream_semantics` checks both stream-producing arguments and
  a stream-producing target before preparation, while stream consumer cleanup
  keeps its own `Flow::ContinueConsumer` path.

These facts were checked with focused `rg` searches over
`eval_context.rs`, `eval_context/{timeout,concurrent}.rs`, `program_db.rs`,
`program_stream.rs`, and `program_invocation.rs`. No production or sibling
test-owner file was modified.

### Validation

```text
focused | cargo test -p skiff-runtime-eval assembly_tail_call_ | V2 |
working tree descended from bc4fea09 | PASS: 9 passed, 0 failed |
canonical assembly positive/fallback matrix

format | cargo fmt --all; cargo fmt --all -- --check | V2 |
same working tree | PASS | scoped Rust formatting
```

The focused build emitted existing workspace warnings; none originated in the
new test module. V2 did not run the runtime selector, I1 combined probe, live
checks, source suite, or full gate. No R1 production defect was found.
