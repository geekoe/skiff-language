# P5-F248 Package call public-path addressing

## Context

After F246, real AIHub reaches Package call-address resolution errors:

- `internal.aihub_service.skiff:1443`: `codexRelay` has no stable key
  `relayProxy.responsesCompletedResult`;
- lines 1566, 1597 and 2032;
- managed provider line 16;
- provider catalog lines 34 and 60.

Several diagnostics indicate dot-form paths where the canonical Package public
path uses slash-form addressing.

## Required investigation and implementation

- Compare every source call with the exact published Package/ServiceContract
  callable public path and stable key.
- Determine whether source syntax should canonically map module/member dots to
  public-path separators or whether these callers contain stale addresses.
- Apply one consistent rule across Package and ServiceContract calls.
- Preserve exact alias, owner, version/build/ABI and stable-key validation.
- Do not add per-call aliases or fuzzy path matching.

## Acceptance

- Positive tests cover nested Package and ServiceContract public paths through
  manifest aliases.
- Dot/slash ambiguity and nonexistent stable keys fail with a diagnostic that
  shows the exact expected public paths.
- Real AIHub crosses all cited calls.
- Relevant compiler/Internals tests, workspace check, result and commit.
- No push, stable operation or disk cleanup.
