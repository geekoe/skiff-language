# P5-F250 Package nominal object lowering target result

## Outcome

- Executable signature projection no longer replaces an exact
  `PackageTypeRef::PackageSchema` with builtin `unknown`.
- File IR receives a `PackageSymbol` carrying the canonical package owner and
  stable schema key. The Package schema type ID remains validated by the
  source/package schema facts that produced that exact signature.
- Object materialization continues to validate fields against the canonical
  record or selected union branch.
- A discriminated-union object is now constructed with its exact Package
  nominal target instead of losing identity to the structural branch record.
- Exact target comparison remains enabled. Missing facts, unrelated Package
  nominals, forged targets, missing/extra fields and stale nested field types
  still fail closed.

## Coverage

The new source-to-FileIR integration fixture builds a real dependent Package
with a `ToolChoice` tagged union and nested nullable `ToolOptions` record.
Both the scalar `auto` branch and the nested `tool` branch lower to
`Construct` expressions whose target is exactly dependency
`tools::ToolChoice`.

Existing object-materialization negatives continue to reject forged map
targets and stale nested/synthetic field types.

## Real AIHub

Using the ABI-consistent store `/tmp/skiff-f247-exact3.yi2dtw`, AIHub crossed
all four `llmApi.LlmToolChoice` constructors:

- `internal/aihub_service.skiff:1198`
- line 1202
- line 1206
- line 1210

The complete AIHub Package build succeeded and emitted Package artifact
`skiff-package-build-v4:sha256:c688042302bb8279c1b107203feb940b354c2abfa2e0241e712ff9882309b8a2`.

The current Internals `service.yml` still uses the removed scalar
`http`/`websocket` authoring form. For this compiler validation those two
validation-only fields were omitted; that manifest edit is not part of F250.
The resulting package build has no further source or File IR lowering blocker.
Its service projection still reports independently owned callable-effect
unavailability for `handleAihubHttp`, `managedLlm.streamChat` and
`managedLlm.validateChat`.

## Validation

- executable Package type projection: 2 passed;
- lowering object materialization: 5 passed;
- Package nominal source-to-FileIR fixture: 1 passed;
- real AIHub Package build: passed;
- workspace check and diff check: passed.

No push, stable-instance operation or disk cleanup was performed.
