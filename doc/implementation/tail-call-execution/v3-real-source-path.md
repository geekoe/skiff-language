# V3 real source tail-call path

Status: ready for integration

Repository: `/Users/geek/workspace/skiff`

Baseline: `bc4fea09685c6e92ed3fb5aa523ebc7cac6ba2df` /
`910bc17d1e752c81ae10a2105a77d89a113a292c`

Integration: `codex/tco-integration` / `/root/tco_integrator`

## Authority and parent

This leaf implements the V3 node from
[`parent-checkpoint.md`](parent-checkpoint.md). The parent traces the canonical
contract to:

1. [`../../architecture/tail-call-execution.md`](../../architecture/tail-call-execution.md);
2. [`../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety`](../../reference/runtime.md#tail-call-execution-and-recursive-stack-safety);
3. the parent checkpoint;
4. this leaf contract and result.

Canonical documents win every conflict. This leaf does not reinterpret tail
position, eligible targets, budget accounting, depth semantics, or lexical
barriers.

## Frozen preflight facts

- R1 production TCO is integrated in the baseline at
  `62db55e1d85e00727faf7d28526dc9515745f4bb`.
- `scripts/lib/skiff-source-test-registry.mjs` canonically maps the `std` entry
  to `test-services/std`.
- `scripts/lib/skiff-source-test-suite.mjs` passes that root to
  `skiff-test-runner` inside one isolated runtime after canonical artifact
  bootstrap. The resulting evidence traverses source parsing and lowering,
  File IR, assembly/linking, and runtime evaluation.
- Test discovery recursively selects files ending in `.test.skiff`, so adding
  the owned fixture requires no registry, selector, or tooling change.
- The baseline has no `test-services/std/tail-call.test.skiff`; no sibling owns
  that path.

## Scope and minimum closure

The only executable write is
`test-services/std/tail-call.test.skiff`. It must:

- contain a direct `return` of an exact local executable call;
- recurse more than the non-tail safety depth of 32;
- terminate through an ordinary branch;
- carry and validate an accumulator so a false early return or lost argument is
  observable.

One direct-self fixture is the minimum sufficient source proof. Generic and impl
source cases remain covered by the parent matrix's C1/V2 owners; duplicating
them here would not strengthen the required real-source-path criterion and
would enlarge the black-box failure surface.

## Non-goals and write boundary

- No production, runtime unit test, compiler, linker, registry, selector, or
  tooling changes.
- No cross-module fixture tree, persisted tail marker, language annotation,
  keyword, manifest field, config, environment variable, or test-only bypass.
- No full gate. V3 exclusively owns the canonical `skiff-tests` selector.

If the source cannot compile/link, or the integrated R1 runtime cannot execute
the eligible call, this leaf records exact stage evidence and reports the
production failure without modifying R1 or expanding ownership.

## Verification

Run in this order:

1. bootstrap the focused temporary artifact root with the same canonical
   `skiff-package-service-smoke-fixture --bootstrap-only` entry used by the
   source suite;
2. focused fixture entry:
   `node scripts/skiff.mjs test test-services/std/tail-call.test.skiff
   --artifact-root <temporary-root> --deny-skips --require-tests`;
3. canonical selector:
   `node scripts/verify.mjs --only skiff-tests`.

The explicit focused CLI requires an existing artifact root containing the
canonical `skiff.run/std` artifact. An empty root correctly stops at package
contract validation before source execution, so the canonical bootstrap is a
required precondition rather than a fixture workaround.

## Result

The focused entry passed after canonical bootstrap:

```text
PASS tail-call.__test::direct tail recursion preserves its accumulator beyond the depth guard
test result: ok. 1 passed; 0 failed
```

The canonical selector then passed the whole isolated source suite:

```text
std: 12 passed; 0 failed
alias-return-catch-once: 7 passed; 0 failed
package-service-host: 9 passed; 0 failed
[skiff-tests] passed 2 canonical source test entries
All selected Skiff verification phases passed.
```

The new source case therefore traversed source compilation, File IR,
assembly/linking, activation, and runtime evaluation, completed 1000 eligible
tail transfers without hitting the non-tail depth fuse, and returned the exact
accumulator value `500500`.

Actual tracked write set:

- `doc/implementation/tail-call-execution/v3-real-source-path.md`;
- `test-services/std/tail-call.test.skiff`.

No production, runtime test, compiler, linker, registry, selector, tooling,
manifest, or config file changed. No full gate was run.
