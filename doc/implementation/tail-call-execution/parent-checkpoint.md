# Tail-call execution parent checkpoint

Status: ready for implementation

Repository: `/Users/geek/workspace/skiff`

Baseline: `2c2cba91f72abc999eaa603681bddce282b26e75` /
`891bcf8a1a64a78eadac117f421012d0c469393b`

Integration: `codex/tco-integration` / `/root/tco_integrator`

## Authority

Leaf tasks must cite this checkpoint. The trace is:

1. architecture authority:
   [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md);
2. user semantics:
   [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety);
3. this checkpoint;
4. leaf task/result.

Canonical documents win every conflict. A leaf must stop rather than reinterpret
their public contract.

## Baseline facts and vertical path

The baseline already has the complete persisted fact chain:

```text
StmtIr::Return -> ExprRefIr -> ExprIr::Call
  -> LocalExecutable or PublicationExecutable
  -> code_linker::link_call_target
  -> LinkedCallTarget::Executable { ExecutableAddr }
```

`RuntimeExecutionProjection` already unifies legacy and assembly target/type
views. `ExecutableInvocation` and `AssemblyExecutableInvocation` already own env,
generic substitution, self, and argument declaration. `ProgramExecutionContext`
already shares execution control, heap limits, Actor frame, correlation, local
stack, and active non-tail depth.

The minimum production closure is one internal prepared frame, one lexical
tail-propagation context, and one loop in `program_execution`. There is no
existing ordinary-call trampoline to reuse.

The canonical production route is:

```text
runtime/host request.start
  -> runtime/request assembly or HTTP gateway execution
  -> runtime/eval ingress
  -> execute_runtime_assembly_addr
  -> call_program_executable[_carriers]
  -> call_assembly_executable
  -> exec_program_executable
  -> EvalContext::{exec_program_block, exec_program_statement}
```

`Interpreter::with_program` is the legacy entry and reaches the same
`program_execution`/`EvalContext` owners. The real authoring probe is
`scripts/run-skiff-tests.mjs` -> isolated runtime -> compiler/File IR ->
assembly/link -> this evaluator -> source assertion.

## Exact owners

### R1 shared runtime production

One owner exclusively closes this unstable surface:

- `runtime/eval/src/env.rs`: internal control result;
- `runtime/eval/src/eval_context.rs`: direct `Return(Call)` recognition,
  left-to-right single evaluation, and propagation context;
- `runtime/eval/src/program_execution.rs`: return-plan comparison,
  implicit/explicit self convergence, non-tail depth push, prepared frame, and
  the only trampoline;
- `runtime/eval/src/flow_completion.rs` and causally required exhaustive `Flow`
  consumers;
- lexical owner sites in `eval_context/{timeout,concurrent}.rs`,
  `program_db.rs`, `program_stream.rs`, and `program_invocation.rs`;
- `assembly_execution/projection.rs` only if existing projection methods cannot
  expose the resolved executable/plan without duplication.

This is an ownership boundary, not a mechanical whitelist. Ordinary block,
`if`, statement `match`, and array/map loop may propagate. Timeout, DB,
concurrent, catch/value wrapper, deferred producer, stream-producing argument,
and stream cleanup must use the ordinary path unless the canonical cleanup seam
is proved.

### S1 independent scheduler depth

`borrow_for_scheduled_task()` already resets depth and local stream producers
already use it. The bounded residual is:

```text
async_stream_cancel.rs::spawn_provider_stream (tokio::spawn)
  -> run_provider_stream
  -> provider_context.borrow()  # inherits depth incorrectly
  -> call_provider_callable
```

S1 owns only this callable entry and its focused evidence. Error-export borrows
remain ordinary. `prepared_unary` and callback waits remain original-chain
continuations and must not reset depth.

### Evidence-only owners

- C1 compiler/linker shape:
  `compiler/lowering/src/source_file_lowering.rs` tests and a small
  `runtime/linker/src/assembly/tests/` module. Prove `Return.value` selects the
  exact call and both local/publication targets normalize; do not change
  production IR/linking.
- V1 legacy/safety/pressure:
  `runtime/driver/eval/tests/program_execution.rs` plus preferably
  `program_execution/tail_call_execution.rs`.
- V2 canonical assembly:
  new `assembly_execution/ordinary/tests/tail_call_execution.rs`, registered by
  `ordinary/tests.rs`.
- V3 real source:
  add only `test-services/std/tail-call.test.skiff`; the existing registry
  already consumes that directory.

Baseline `recursive_executable()` is direct `Return(Call)` with budget disabled.
V1 must make it genuinely non-tail; otherwise TCO turns the old guard test into
a non-yielding infinite loop.

## DAG and batches

| Node | Ready / blocked by | Non-overlapping write owner | Unblocks |
| --- | --- | --- | --- |
| P0 checkpoint | ready; authority integrated | this file | R1, S1, C1 |
| R1 shared evaluator | P0 | shared runtime production above | V1, V2, V3 |
| S1 scheduler reset | P0 | provider stream entry/test | I1 |
| C1 structural evidence | P0 | compiler/linker tests | I1 |
| V1 legacy/pressure | R1 integrated commit | driver tests | I1 |
| V2 assembly matrix | R1 integrated commit | assembly test module | I1 |
| V3 source path | R1 integrated commit | one `.skiff` fixture | I1 |
| I1 combined probe | all implementation/evidence nodes merged | integrator only | F0 |
| F0 preflight/freeze | I1 pass | gate owner; no verdict | A1, G1 |
| A1 acceptance | frozen F0 commit | independent read-only verdict | completion |
| G1 final gate | frozen F0 commit | unique gate owner | completion |

Batch 1 integrates P0, then runs R1/S1/C1 in parallel. R1 is the critical path
and must not be split by individual `Flow` consumers. After R1 integrates,
Batch 2 fans out V1/V2/V3. I1 then forms one pre-acceptance checkpoint.

## Completion evidence and masking

| Criterion | Evidence | Owner |
| --- | --- | --- |
| one legacy/assembly loop | deep result in each projection; one production trampoline by search | V1, V2, A1 |
| direct/mutual/cross-module/generic/impl/branch | Return-to-call structure plus assembly results | C1, V2 |
| arg order, generic/self, heap carrier, return plans | once/ordered args; equal carrier; unequal plan ordinary fallback | V2 |
| lexical/target negatives | binary/wrapper/argument/catch/timeout/concurrent/DB/stream/service/Actor/native | V1, V2 |
| non-tail fuse | depth 32 enters; next frame reports `programCallDepth`; runtime stays healthy | V1 |
| budget/deadline/stop | infinite tail ends as `instructionLimitExceeded`, not depth; finite hop accounting | V1 |
| stack bounds | 100,000-hop result on actual 1 MiB Tokio worker; 100,000-hop error stack bounded | V1 |
| scheduler depth | seeded non-zero caller depth reaches provider callable from fresh task depth | S1 |
| real source chain | canonical isolated `skiff-tests`, recursion greater than 32 | V3 |
| no duplicate/persisted mechanism | diff/search excludes marker, second loop, bypass, keyword, config | C1, A1 |

The 1 MiB workload must run inside `tokio::spawn` on a one-worker Tokio runtime;
polling only the `block_on` root future does not prove worker-stack use.

A lowering/link failure masks cross-module runtime evidence; an R1 compile or
ordinary-return failure masks all black-box lanes. Source compile/link failure
must be classified before changing runtime again.

## Verification and gate plan

Leaf owners run only their focused filters. Unique selector ownership is:

```text
C1: node scripts/verify.mjs --only compiler
R1: node scripts/verify.mjs --only runtime
V3: node scripts/verify.mjs --only skiff-tests
S1/V1/V2: non-overlapping Cargo test filters only
```

I1 owns this cheap merged-state probe; leaf contracts may replace the provisional
test filter names with their final names:

```bash
cargo check -p skiff-runtime-eval -p runtime -p skiff-compiler-lowering -p skiff-runtime-linker
cargo test -p runtime runtime_program_recursion_fails_before_exhausting_the_worker_stack
cargo test -p skiff-runtime-eval tail_call_projection_parity
```

F0 maps every criterion above to merged evidence, then runs non-verdict preflight:

```bash
node scripts/verify.mjs --only compiler --list
node scripts/verify.mjs --only runtime --list
node scripts/verify.mjs --only skiff-tests --list
node scripts/verify.mjs --list
```

F0 also records exact HEAD/tree, clean worktree, `node`/`pnpm`/`cargo`, source
root, cache/target owner, and absence of competing full gates or shared-state
mutation. Preparation changes happen before freeze. A1 gives one high-risk
PASS/FAIL on the frozen commit. G1 runs `pnpm verify` exactly once.

## Non-goals and stop conditions

- No artifact/File IR marker, metadata convention, SCC annotation, schema,
  keyword, manifest field, environment variable, config, or compatibility path.
- No second trampoline, assembly-to-legacy conversion, heap continuation, or
  ordinary tail-call `tokio::spawn`.
- No TCO for `PackageDirect`, dynamic interface/const receiver, service, Actor,
  callback, native/builtin, stream defer, spawn, or emit.
- No non-tail optimization, depth-constant change, VM-stack rewrite, or change
  to budget/deadline/stop, heap, Actor, or stream terminal semantics.
- No new general checker: focused structure tests plus frozen diff/search cover
  the risk without a new registry or unstable symbol-name contract.

Return `TASK_SCOPE_EXPANDED` for a public contract change, persisted marker,
second lifecycle/trampoline owner, or sibling write collision. Mechanical
exhaustive-match and fixture closure inside R1 is expected and belongs in its
leaf contract.
