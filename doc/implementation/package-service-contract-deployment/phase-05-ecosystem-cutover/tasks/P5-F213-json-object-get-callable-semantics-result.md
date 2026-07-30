# P5-F213 JsonObject.get callable semantics result

## Result

Completed.

The Runtime stores nested JSON objects and arrays as request-heap handles.
`JsonObject.get` clones only the `RuntimeValue` shell:

- a missing field returns `null`;
- scalar fields return scalar values;
- nested objects and arrays return the original heap handle, not a deep copy.

The exact audited semantics for `receiver:JsonObject.get@1` are therefore:

- no caller-reachable write;
- may return a receiver-reachable alias;
- no thrown alias or caller-value escape;
- same-heap identity is required when the receiver is caller-reachable;
- no unknown target and no suspension;
- return provenance is caller parameter 0, the receiver.

The source transfer remains contextual. For a fresh receiver, the return
provenance maps to `Fresh` and the same-heap effect is not caller-observable.
This is important for the real ChatGPT Plan codec: `jsonObject` serializes and
decodes its input before `jsonField` calls `get`. Its receiver is therefore
fresh. The focused `jsonField -> claimsFromJwt` shape has no callable effects
and has only detached `Fresh`/`Constant` return provenance, with no caller
parameter provenance.

The old compiler-local `JsonObject.get` fallback was removed. The canonical
audited receiver registry is now the sole semantics owner.

## Signature and fail-closed coverage

`JsonObject.get` is non-generic and has the canonical source signature:

```text
JsonObject.get(string) -> Json
```

Source validation now enforces its single string key, matching the existing
canonical receiver operation and fixed return type. Tests reject:

- a non-`JsonObject` receiver;
- a missing or extra argument;
- a non-string key;
- use as an incompatible return type.

The structured receiver operation continues to reject a wrong canonical key,
receiver/method pair, or signature version through the shared exact operation
decoder.

## Runtime and compiler coverage

Runtime-backed tests cover missing, scalar, nested object, and nested array
fields and assert the exact original nested heap handles.

Compiler tests cover both sides of the conditional value behavior:

- direct `JsonObject.get` from a caller-owned receiver retains
  `returnsCallerAlias`, `requiresSameHeapIdentity`, and receiver provenance;
- the real codec-shaped fresh materialization removes caller-observable alias
  and heap requirements without erasing the actual direct semantics.

## Real ecosystem acceptance

An isolated artifact store was bootstrapped with canonical `std`, then the real
`llm-api`, `llm-providers`, `agent`, and Relay sources from
`internals-p5-f188` were authored with this Skiff worktree. The shared stable
instance was not used.

The original `jsonField -> receiver:JsonObject.get@1` unknown-target leaf is
resolved by the exact registry and the focused real caller shape is detached.
The ecosystem then records the following next independent exported blocker:

- `chatgptPlan.responses` remains analyzed with all seven may-effects set,
  `unknownCallTarget` provenance, and no published resolved-call targets;
- Relay `v1Proxy` remains unavailable for `unknownEffect`,
  `unknownCallTarget`, `writesCallerReachable`, `returnsCallerAlias`,
  `throwsCallerAlias`, and `requiresSameHeapIdentity`.

Those facts are no longer caused by `JsonObject.get`; the package artifact does
not publish the internal unresolved leaf for `responses`, so narrowing that
separate summary requires a subsequent audit rather than a Relay-specific
exception here.

## Verification

- `cargo test -p skiff-artifact-model --lib --no-fail-fast`: 117 passed.
- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: passed.
- focused compiler signature positive/negative test: passed.
- focused Runtime receiver tests: 6 passed.
- `cargo check --workspace`: passed.
- `git diff --check`: passed.

