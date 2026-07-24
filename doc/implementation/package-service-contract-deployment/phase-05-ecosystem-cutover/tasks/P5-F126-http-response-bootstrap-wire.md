# P5-F126 HTTP response bootstrap wire

## Authority

- Canonical design: `doc/architecture/package-service-contract-deployment.md` §12.
- Entering wire checkpoint: F103/F106/F107.
- Confirmed config:
  `http: { port, maxRequestBytes, maxResponseBytes }`, with both byte values required and instance-wide.

## DAG node

Extend the canonical Router→Runtime connection bootstrap with the instance-wide HTTP response byte ceiling.

## Write scope

- Canonical TypeScript/Rust runtime bootstrap frame/codecs.
- Shared cross-language corpus and focused transport tests.

Do not modify Router config/enforcement, Runtime response execution, service manifests, source repos or stable.

## Required wire

```text
router.bootstrap {
  artifactsPath,
  serviceDb: { mongoUrl },
  http: { maxResponseBytes }
}
```

- `maxResponseBytes` is a required positive safe integer.
- `maxRequestBytes` is Router-only and must not appear on the Runtime wire.
- Missing/zero/fractional/overflow/unknown fields fail closed.
- Duplicate/conflicting bootstrap remains rejected.

## Acceptance

- TS/Rust positive fixture and strict negative corpus agree.
- Cross-language transport tests and `git diff --check` pass.

Risk: high, cross-process wire. No merge/push/stable.
