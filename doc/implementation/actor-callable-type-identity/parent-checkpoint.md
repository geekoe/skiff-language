# Actor callable canonical dependency type identity checkpoint

> Status: cross-repository implementation checkpoint for S1. This document
> records frozen evidence and execution boundaries; it does not add or revise
> actor semantics. Skiff actor semantics remain owned by
> [`../../architecture/actor-model.md`](../../architecture/actor-model.md).

## Frozen identity and parent evidence

- Skiff repository baseline commit/tree:
  `2b584edc6c30cec4acf8fd2c9e3bde85790fe234` /
  `d6e2bb3f62d48e10810b688791833cb22d189354`.
- Cross-repository Internals integration parent commit/tree:
  `bf2546456b045138ac6641d55b4b326bd58f99ef` /
  `7b53939cc1267fca6bfd262e65c56fa06dd9701e`.
- Direct blocking result commit/tree:
  `3bc9ea16ffec2015ce479ce0efca20212e3f8d0e` /
  `857154455251648ec2c0b2b357486d929cfd76a8`; the result commit is reachable
  from the Internals parent and owns
  `packages/agent/docs/implementation/agent-tests-suite/tasks/p1x-tool-result-continuation.md`.
- The cross-repository product behavior chain remains
  `internals/packages/agent/docs/test-suite-design.md` to
  `internals/agine/docs/thread-actor-drain-design.md`. It requires the existing
  `ThreadActor.onToolResult` public actor call to persist first and then spawn
  the next tick. This checkpoint does not change that behavior.
- Integration branch/worktree:
  `integration/actor-callable-type-identity` /
  `/Users/geek/workspace/skiff-actor-callable-type-identity`.

## Confirmed defect boundary

The direct P1X result establishes an opposing canonical identity at one actor
call boundary:

1. The callable exposed to the cross-package caller statically expects the
   local test-service projection, `subjectImpl/tools.ToolResult`.
2. The actor method body and its direct dependency call decode/use the package
   dependency identity, `subject/tools.ToolResult` (the
   `agine.ai/agent` dependency export).
3. The two values report the same local ABI hash, but retain different
   canonical package identities. Supplying the local identity therefore
   compiles at the actor call site and fails during runtime argument decoding
   with request-local `Nominal(PlatformBuiltin(JsonDecode))` before the actor
   method body begins.
4. Supplying the dependency identity reverses the failure: the direct method
   body/dependency call accepts it, while the actor callable call site rejects
   it statically because its projected signature expects the local identity.

The P1X diagnostics replaced the entire `onToolResult` body with a static
receipt and still reproduced the runtime decode failure. Removing
`spawn self.tick()` and all observations after the call also reproduced it.
Consequently, the failing owner is not Internals settlement, continuation,
spawn scheduling, scripted LLM state, or post-call assertions. Internals
production and focused fixtures remained byte-for-byte unchanged in the P1X
result.

## Required closure

The minimum real vertical closure is a cross-package `kind: test` service:

```text
dependency package B exports a nominal record T
  -> package A declares an actor method whose parameter is B.T
  -> package C obtains A's actor through its package/test projection
  -> C passes the matching exported B.T value
  -> compile and link preserve one canonical dependency identity
  -> runtime actor dispatch decodes the argument and enters the method body
  -> the body produces an observable result
```

The regression must exercise the real package publication/projection, linked
actor dispatch, and runtime argument decoder. A unit-only hand-built
`RuntimeTypePlan`, an actor method with a package-local parameter, or a test
that stops after artifact inspection is insufficient.

## Owner and implementation boundary

The shared owner is the existing actor callable ABI pipeline:

- producer: compiler package-artifact actor public-method/callable projection
  and its canonical type references;
- consumers: cross-package source typing/linking of that projected callable
  and runtime actor-method argument decoding from the linked public-method
  plan.

The implementation task must identify the first point where the actor callable
parameter loses or substitutes the dependency package identity, then repair
that existing projection/consumption path. It must keep one canonical
PackageArtifact actor representation, as required by the actor model's
"消费视图与验收矩阵" section; it may not introduce a second actor signature,
test-only rewrite, or runtime decode bypass.

## Boundaries and non-goals

- Do not change actor source syntax, public behavior, method signatures,
  canonical nominal-type validation, or package dependency semantics.
- Do not add a manifest field, schema, configuration, language keyword,
  compatibility path, or centralized side channel.
- Do not weaken the direct-call identity mismatch: non-equivalent canonical
  package identities must continue to fail closed at compile/link time.
- Do not modify Internals settlement, ThreadActor behavior, spawn semantics, or
  agent tests in this Skiff task.
- Do not broaden into actor ownership, lifecycle, routing, concurrency,
  tracing, or return-value ABI changes.
- No full gate, stable instance mutation, live chat smoke, or push belongs to
  this implementation checkpoint.

## Risk probes and validation levels

Risk is high because actor public method signatures are part of actor ABI and
are consumed across compiler, linker, and runtime views.

1. **Static projection probe:** a dependency-exported nominal parameter has
   the same canonical type identity in the actor public method and the method
   executable/body. Also retain a negative case proving a distinct nominal
   package identity is rejected.
2. **Compiler/linker probe:** package C compiles and links a call to package
   A's actor method with package B's exported value without local/dependency
   identity substitution or unresolved symbols.
3. **Runtime actor ABI probe:** the minimal real cross-package test service
   invokes the actor method, runtime decoding succeeds, and an assertion from
   inside/after the body proves the body was entered.
4. **Consumption-view check:** cover the actor model's relevant package public
   and `kind: test`/`topLevelAlias` views; do not claim closure from a same-file
   or same-package-only actor test.
5. **Downstream release probe:** after this Skiff checkpoint is integrated,
   rerun the Internals P1X public
   `onUserMessage -> pending tool -> onToolResult` probe. That downstream
   probe remains owned by Internals and is not part of the Skiff implementation
   branch's focused validation.

Evidence is valid only for the exact result commit/tree, its test fixtures,
and the compiler/runtime source identity used by the command. Changes to actor
public-method projection, linked type plans, runtime argument decoding, or the
cross-package fixture invalidate the corresponding level.

Stop and report `TASK_SCOPE_EXPANDED` if a fix requires new public semantics,
a manifest/schema/config change, relaxed canonical identity validation, an
Internals behavior change, or more than the existing actor callable ABI owner
and its direct consumers.
