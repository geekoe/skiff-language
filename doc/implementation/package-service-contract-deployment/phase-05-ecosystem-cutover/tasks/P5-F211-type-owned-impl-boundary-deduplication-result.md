# P5-F211 Type-owned impl boundary deduplication result

## Result

Completed.

Publication API construction no longer turns every impl method owned by a
public nominal type into an independent public callable. A public type remains
a public symbol and package schema owner, but service operations now come only
from explicit publication facts:

- explicitly published free functions remain independent callables;
- methods reached through an explicitly published instance and its declared
  interface remain public instance callables;
- a nominal type's otherwise-unpublished impl methods remain implementation
  details.

The impl-method selector diagnostic now directs authors to an explicit public
instance instead of suggesting that publishing the receiver type publishes its
methods.

## Ownership and fail-closed coverage

Source tests cover:

- a public nominal type with an impl method and no public instance, producing
  no callable for that method;
- the same nominal implementation reached through one explicit public
  instance, producing exactly `handler.submit` and no `Handler.submit`;
- an explicitly published free function alongside the instance method,
  preserving both distinct operations;
- similarly named free functions at different explicit public paths,
  preserving both source-owned operations.

Existing public-instance validation continues to reject duplicate validated
methods, conflicting interface ownership, missing implementation methods and
invalid executable links. The implementation does not deduplicate by display
name.

## Fresh Relay acceptance

The real Relay source in `internals-p5-f188/codex-relay/service` was compiled
from the current checkout against an isolated temporary canonical artifact
store. The store was freshly seeded with the current official std package and
fresh `agine.ai/llm-api` and `agine.ai/llm-providers` publications. No shared
stable instance was used.

The resulting Relay package artifact contains exactly 17 callable links.
Both unintended type-owned paths are absent:

```text
CodexRelayProxy.responsesCompleted
CodexRelayProxy.responsesCompletedResult
```

The explicitly published instance still owns both intended operations:

```text
relayProxy.responsesCompleted
relayProxy.responsesCompletedResult
```

The public instance Local ABI method map contains exactly those two methods.
Authoring proceeded through package and ServiceContract record generation, then
stopped at the existing independent deployment blocker:

```text
ingress operation adminChatgptOauthStart is boundary unavailable
```

## Verification

- `cargo test -p skiff-compiler-source --lib --no-fail-fast`: 252 passed.
- `cargo test -p skiff-compiler-projection -p skiff-compiler-contract --no-fail-fast`:
  27 passed.
- `cargo check --workspace`: passed.
- fresh Relay callable-link count: 17.
- `git diff --check`: passed.

Nothing was pushed and the shared stable instance was not operated.
