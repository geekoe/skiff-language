# P5-F76 Contextual callable provenance

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact callable effects and
  fail-closed package boundary.
- Predecessor: D65 proved facts handoff is exact; pollution begins in compiler/source transfer.
- Worktree: create `skiff-p5-f76-contextual-provenance` from current Skiff integration.
- Write owner: compiler/source callable provenance abstract values, SCC/wrapper composition and
  focused compile-only tests.
- Required outcome: discharge `RequiresSameHeapIdentity`/caller-write effects when `Array.push`
  receiver is proven function-local fresh, while caller-owned receiver remains W+I. Internal
  return/throw unknown provenance must only make the public wrapper fail closed when it can reach
  caller-visible return/throw/escape lanes; do not globally erase genuine unknown targets.
- Cover fresh local push, caller-owned push, HTTP/config/throw wrapper, dependency wrapper,
  multi-hop SCC and suspend. Preserve unresolved/dynamic full fail-closed behavior.
- Combined validation owner: compiler/source focused graph tests plus one compile-only test-runner
  loop for real `aliyunoss`, `track`, `openai`, `http-session` production+overlay, asserting case0
  facts without runtime/router/Mongo.
- Do not edit artifact model/projection/boundary/test-runner production logic, packages, stable,
  merge, push, compatibility, or full gate. Deliver one commit/evidence.

