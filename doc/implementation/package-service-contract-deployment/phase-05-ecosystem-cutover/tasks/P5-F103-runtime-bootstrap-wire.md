# P5-F103 Router/Runtime bootstrap wire

## Authority

- Canonical design:
  `doc/architecture/package-service-contract-deployment.md`
- Relevant clauses: §11, §12 and §13.
- Audit input: P5-D77.

## DAG node

Define the single connection-scoped Router→Runtime bootstrap control shared by the later Router and Runtime
consumer migrations.

## Write scope

- Canonical TypeScript and Rust runtime transport frame definitions/codecs.
- Cross-language corpus/fixtures and focused transport tests.

Do not wire Router server/config, Runtime resolver/config, activation state, artifact loading, compiler,
Registry, or stable instance.

## Contract

The Router sends exactly one bootstrap before any Runtime activation or registration:

```text
router.bootstrap {
  artifactsPath: absolute normalized string,
  serviceDb: { mongoUrl: non-empty string }
}
```

- Both values are required and singular.
- Duplicate identical bootstrap and any conflicting bootstrap are rejected; consumers may enforce the
  stricter exactly-once form.
- Activation/register before bootstrap is rejected by consumers.
- This replaces activation prepare/commit `serviceDb` carriage and legacy `router.control.artifactRoots`;
  consumer removal belongs to later tasks.
- No Registry endpoint or identity appears in this frame.

## Acceptance

- TS and Rust strict codecs accept one canonical positive fixture.
- Missing/empty/relative artifactsPath, missing/empty mongoUrl, unknown fields and plural artifact roots fail.
- Cross-language corpus parity passes.
- `git diff --check` passes.

Risk: high, cross-process wire. Candidate after completion: shared implementation checkpoint. No push or
stable-instance operation.
