# P5-F248 Package call public-path addressing result

## Decision

The compiler rule is already correct and remains unchanged:

- dependency callable syntax is `alias/publicPath(...)`;
- `.` inside the address belongs to the published nested public path, as in
  `codexRelay/relayProxy.responsesCompletedResult(...)`;
- `alias.member(...)` is ordinary member access and is not accepted as a
  dependency call;
- qualified dependency types continue to use `alias.Type`.

The cited `llmProviders` calls were stale callers. No fuzzy matching,
per-call alias or dot-to-slash compiler fallback was added.

## Internals change

Internals commit `5861c13` changes ten AIHub production and test call sites
from dot form to the exact published Package paths:

- `llmProviders/provider`
- `llmProviders/fallbackModels`
- `llmProviders/streamChat`
- `llmProviders/validateRequest`
- `llmProviders/requestBody`

The reverse search for `llmProviders.<callable>(...)` in AIHub is empty.

`codexRelay/relayProxy.responsesCompletedResult(...)` was already correctly
written. Its earlier failure came from validating against an old
ServiceContract that did not contain the public-instance method operation.

## Validation

Existing compiler coverage passed:

- syntax dependency-address parsing and nested slash normalization: 2 passed;
- compiler nested Package public-path preservation through a manifest alias:
  1 passed;
- existing source diagnostics continue to reject dot-form dependency calls
  and show the exact slash spelling.

Real validation used one ABI-consistent artifact store,
`/tmp/skiff-f247-exact3.yi2dtw`, containing the F247 std, llm-api and
llm-providers publications. Relay was then published into the same store.
Its resulting ServiceContract contains:

- `relayProxy.responsesCompleted`
- `relayProxy.responsesCompletedResult`

Both are `available`; the latter has operation identity
`skiff-contract-operation-v1:sha256:51fa082dd0d33b09f45e4900805c28801cb3108b4eac813697e66e5f8a6b007d`.

The subsequent real AIHub build crossed:

- `internal/aihub_service.skiff:1443`;
- lines 1566, 1597 and 2032;
- `internal/managed_provider_transport.skiff:16`;
- `internal/provider_catalog.skiff:34` and 60;
- the analogous test call sites.

No missing public path, missing operation stable key, or dot-form dependency
call diagnostic remains.

## Next blocker

AIHub now reaches an independent File IR lowering failure:

`internal/aihub_service.skiff:1198`, in
`autoToolChoice() -> llmApi.LlmToolChoice`.

The object literal `{ tag: "auto" }` has a target-typed materialization fact
for exact dependency Package symbol `llmApi.LlmToolChoice`, but lowering sees
the current expected target as builtin `unknown` and rejects the mismatch.
The same adjacent constructors at lines 1202, 1206 and 1210 should be audited
with that fix.

Relay validation temporarily used the already-known F188 diagnostic workaround
for the unrelated `chatgpt_oauth.skiff:131` Package record field projection;
that workaround was not included in the F248 implementation commit.

No push, stable-instance operation or disk cleanup was performed.
