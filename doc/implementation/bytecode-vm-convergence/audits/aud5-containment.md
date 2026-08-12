# AUD5: capability ledger and containment

> Status: completed

## 1. Current reachability

The current bytecode VM can dispatch scalar/local code, aggregate mutation,
throw/unwind, host effects, streams, service/actor/interface/callback
invocations, and async Pending resumption. The architecture review identifies
which of those lanes are currently correct enough to accept.

## 2. Ledger

| Capability | Current reachability | Current state | Phase owner | Phase 0 action |
| --- | --- | --- | --- | --- |
| scalar/local execution | audit | reachable; VCP success available | Phase 1 | retain candidate path |
| aggregate/lifecycle | audit | enabled-unaccepted | Phase 2 | fail closed in Phase 1 |
| throw/catch/unwind | audit | enabled-unaccepted | Phase 3 | fail closed in Phase 1 |
| Pending/session ownership | audit | mixed | Phase 4 | prevent new use |
| HTTP/resource/stream | audit | enabled-unaccepted | Phase 5 | contain |
| task/service/interface/callback/Actor | audit | mixed/unsupported | Phase 6 | one gate per lane |
| GC/performance | audit | planned/latent | Phase 7 | no premature enablement |

## 3. Production ingress coverage

Production ingress includes HTTP unary, server stream, WebSocket connect/close/
JSON-RPC, task, Actor invocation, and service-to-service call paths. The
containment decision is: Phase 1 accepts only unary scalar operation/gateway
requests that compile without aggregate mutation, throw, host effect, stream,
task, service, Actor, interface, or callback use. Any request outside that lane
fails at the request boundary or compiler admission before Phase 1 production
implementation starts.

No urgent containment code was required for Phase 0 because Phase 1 is not yet
enabled as a production claim; the harness keeps the unsupported request mode
negative case as a permanent fail-closed regression.
