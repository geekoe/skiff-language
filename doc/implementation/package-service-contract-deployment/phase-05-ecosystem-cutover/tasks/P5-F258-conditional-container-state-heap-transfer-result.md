# P5-F258 Conditional container state heap transfer result

## Outcome

Callable-effect analysis now keeps container identity separate from values
returned by `Map.get`. A nullable lookup receives its own local candidate
allocation instead of reusing the Map receiver root. Fresh reference fields
projected from newly materialized records likewise retain an exact local
candidate.

Field stores now transfer over every local Fresh candidate. A mixed
formal/Fresh base performs both parts of the conditional transfer:

- weakly update every local candidate root;
- emit the formal-indexed caller store, write and heap-identity facts.

The same transfer is supported through local callee summaries. `Map.set` records
an exact edge from a local Map to the inserted value, so a mutated local record
may be reinserted into its owning Map.

Before adding any local heap edge, the evaluator walks the existing Fresh graph.
Direct self-storage and transitive cycles remain `unsupportedHeapStore`.
Unknown bases or values, mutated values stored into Arrays or databases, and
other unsupported aliases remain fail-closed. The lattice remains finite:
candidate roots are AST preorder allocation sites and all joins are set unions.

## Real package chain

The unmodified F251 business sources were published with this compiler against
the isolated artifact store `/tmp/p5-f258.zgc7LL/store`.

`agine.ai/llm-api` package build:

```text
skiff-package-build-v4:sha256:64d055eb7f755cd8fccee2bf1827b24b8f1357e5224276140ff7cabbe022e686
```

`decode.decode` is analyzed without `unsupportedHeapStore`. Its exact effects
contain only `maySuspend`; return provenance is Constant and throw provenance
is Fresh.

`agine.ai/llm-providers` package build:

```text
skiff-package-build-v4:sha256:5f7d651c4c4078467243b1387d4c660db2d16866aa80ba0c9585b53870dc69da
```

Both `streamChat` and `chatgptPlan.streamChat` are analyzed and no longer carry
the propagated heap-store Unknown.

AIHub then reaches the next independent exact validation failure:

```text
WebSocket ingress operation must not suspend
```

Thus the llm-api heap-store blocker is cleared through providers and up to
AIHub ingress. The next task must decide whether WebSocket ingress may suspend
or whether its stream path must be represented differently; this result does
not change the business wrapper or relax that rule.

## Validation

- compiler source suite: 284 passed;
- callable-effect focused suite: 75 passed;
- artifact-model and semantic-fact wire suite: 128 passed;
- workspace check: passed;
- real llm-api publish: passed;
- real llm-providers publish: passed;
- real AIHub publish: reached the next exact WebSocket suspension blocker.

No business-source wrapper, push, stable-instance operation or disk cleanup was
performed.
