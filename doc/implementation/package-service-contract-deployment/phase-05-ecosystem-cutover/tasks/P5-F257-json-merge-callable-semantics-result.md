# P5-F257 std.json.merge callable semantics result

## Outcome

The exact native binding
`std.json.merge(Json, Json) -> Json` now publishes audited callable semantics:

- successful return provenance is Fresh;
- no caller-reachable write or returned caller alias;
- no throw alias or caller-value escape;
- no same-heap requirement, unknown target or suspension.

The semantics entry is keyed only by the canonical binding. Registry validation
requires the exact non-generic two-parameter Json signature, Json return type
and registered Runtime handler. Aliases, lookalikes, wrong arity and malformed
parameter or return types remain rejected.

Runtime behavior is unchanged: object/object performs a shallow overlay, null
overlay clones the base, and any other overlay clones and replaces the base.
Tests additionally prove that nested values in the returned tree share no
mutable identity with either input tree.

## Real AIHub

AIHub was published with this compiler from the F251 diagnostic source against
an isolated copy of `/tmp/p5-f251-existing.50R3Sj/store`. The unmodified
diagnostic source produced Package build:

```text
skiff-package-build-v4:sha256:9779dd0dfd8fe2f55bc44a60566d676fc80d46533f3c4509394d1f7deeb70164
```

`internal.aihub_service.applyProviderOptions` cleared
`unknownCallTarget`, and the unknown no longer propagates from
`std.json.merge`. `managedLlm.validateChat` consequently cleared both
`unknownEffect` and `unknownCallTarget`; its remaining boundary reasons are the
independent caller write and same-heap effects.

HTTP and WebSocket ingress still reach a later dependency blocker:

```text
llmChatBodyTextFromOpenAi
  -> llmProviders/requestBody
  -> dependency callable provenance unknown(unknownCallTarget)
```

The exact `agine.ai/llm-providers:requestBody` artifact fact remains
conservatively unavailable with the full unknown effect set. This is no longer
caused by `std.json.merge`.

## Validation

- artifact-model suite: 128 passed;
- Runtime native suite: 95 passed;
- compiler source suite: 283 passed;
- focused semantics/compiler/Runtime tests: 6 passed;
- real isolated AIHub publish: passed and exposed the next dependency blocker;
- workspace check and diff check: passed.

No push, stable-instance operation or disk cleanup was performed.
