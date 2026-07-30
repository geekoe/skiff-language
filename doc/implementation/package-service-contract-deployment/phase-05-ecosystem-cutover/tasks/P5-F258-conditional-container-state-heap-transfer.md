# P5-F258 Conditional container state heap transfer

## Context

AIHub ingress `unsupportedHeapStore` propagates from exact llm-api decoder
facts, not ingress code.

Responses path:

- `decode.skiff:543`, `state.name = name`;
- `state` is either `states.get(key)` (formal parameter 1 reachable) or a new
  Fresh `ResponseToolCallState` (allocation root 38);
- the join contains caller root 1 plus Fresh root 38 and is rejected before a
  formal-indexed store summary can be formed.

Chat-completions path:

- `decode.skiff:410`, `state.name = nextToolName`;
- `state` is either `toolStates.get(key)` from local Fresh Map root 18 or a new
  Fresh record root 296;
- the join has multiple local Fresh candidates and is rejected;
- reinserting the mutated record into the same Map at line 426 is also rejected.

Current analysis conflates a Map receiver allocation with a `Map.get` value
allocation, supports only pure formal or pure single-Fresh bases, and cannot
represent conditional stores.

## Required implementation

- Model container lookup results separately from the container receiver root.
- Track may-points-to candidate roots through nullable lookup, narrowing and
  constructor branches.
- If every candidate is a local Fresh record, apply a weak field store to each
  candidate safely.
- For mixed formal + Fresh candidates, emit conditional formal-indexed store
  transfer while retaining the local Fresh store.
- Support reinserting a mutated local record into its owning local Fresh Map
  with an exact heap edge.
- Detect and reject genuine self-containing/cyclic graphs and unknown container
  aliases.
- Preserve fixed-point convergence, branch joins and fail-closed behavior.

## Acceptance

- Focused fixtures reproduce roots 14/18/38/296 shapes: nullable Map lookup,
  new record fallback, field mutation and Map reinsertion.
- Mixed formal/Fresh, all-Fresh and genuine cyclic/unknown negative matrices.
- Exact llm-api `decode.decode` becomes analyzed without unsupportedHeapStore.
- Rebuild llm-providers and AIHub; `managedLlm.streamChat`, HTTP and WebSocket
  ingress clear this propagated Unknown or expose the next exact effect.
- Compiler source/effect tests, artifact summary round-trip, workspace check,
  result and commit.
- No business-source wrapper, push, stable operation or disk cleanup.
