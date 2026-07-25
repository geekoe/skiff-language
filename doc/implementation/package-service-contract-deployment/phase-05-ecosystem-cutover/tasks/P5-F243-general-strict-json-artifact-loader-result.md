# P5-F243 General strict JSON artifact loader result

Status: complete.

## Root cause

The captured FileIR record was valid UTF-8 and valid JSON without duplicate
keys. Its first rejected token was the decimal point in:

```json
{"kind":"number","value":0.1}
```

`FilesystemRuntimeAssemblySnapshotLoader` used the activation protocol parser
for every artifact record. That parser intentionally accepts only canonical
unsigned safe integers because activation JSON numbers are generations. It
therefore rejected valid FileIR number literals.

## Implementation

- Added a general strict JSON parser for immutable artifact records.
- Preserved strict UTF-8 decoding, duplicate-key detection, surrogate
  validation, trailing-input rejection, JSON number grammar and safe-integer
  checks.
- General records now accept negative numbers, fractions and exponents.
- Kept the activation parser separate and unchanged, so generation syntax
  remains canonical, unsigned and within `Number.MAX_SAFE_INTEGER`.
- Reused the general parser for `runtimeAssembly request.start` JSON rather
  than retaining two general strict JSON implementations.
- Switched only the filesystem artifact loader to the general parser.

## Verification

- Strict JSON and filesystem loader tests: 19/19 passed.
- Router type-check: passed.
- Router full suite: 567/568 passed. The existing Actor production-routing
  test failed independently with `unknown Actor invocation invoke-1` and a
  timeout; it reproduces when run alone and does not use the changed parser or
  loader.
- Relay full service crossed the formerly rejected FileIR and executed all
  tests. The run reached 32 independent behavioral failures; no
  `record is not strict JSON` error remained.
- `git diff --check`: passed.

No stable instance was operated and nothing was pushed.
