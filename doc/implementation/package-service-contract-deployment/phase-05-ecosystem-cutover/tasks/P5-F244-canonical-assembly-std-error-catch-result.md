# P5-F244 Canonical assembly std error catch result

## Outcome

Canonical assembly execution now preserves the exact Package identity mapping in
canonical code-slot order. A linked type address owned by the exact
`skiff.run/std` Package can therefore resolve through its loaded file and type
declaration to the registered builtin error identity.

Catch leaves retain both identities:

- the exact linked address, for nominal Package matching;
- the registered builtin std symbol, for native Runtime error payload matching.

Explicit throws of those linked std error types use the same builtin identity.
No module/type-name guess is made for another Package, and Package types are not
generally equated with builtins.

## Coverage

Canonical linked-address coverage includes every error declaration in the std
Package which has a registered native error identity:

- `std.bytes.DecodeError`
- `std.number.DecodeError`
- `std.json.DecodeError`
- `std.db.ConflictError`
- `std.db.DecodeError`
- `std.file.FileError`
- `std.resource.ResourceError`
- `std.time.DecodeError`
- `std.service.ProviderUnavailableError`
- `std.service.ProtocolError`
- `std.http.HttpError`

The remaining registered builtin-only errors (`CancelError`, `TimeoutError`,
and `config.DecodeError`) remain native and do not use a Package-name fallback.

Negative coverage verifies that a JSON catch does not match the bytes error and
that a non-std Package with identical module/type names retains only its nominal
linked address.

## Real Relay receipt

The real file suite was run with this worktree's compiler and Runtime against
the retained isolated artifact store:

```text
node scripts/skiff.mjs test \
  /Users/geek/workspace/internals-p5-f188/codex-relay/service/relay_responses_projection.test.skiff \
  --artifact-root /tmp/skiff-f232-final.EX7ZAG/store
```

It used a dynamically allocated isolated Router/Runtime/MongoDB instance and
did not operate stable.

The suite reached 22 passes out of 23 tests. All previously uncaught
`std.bytes.DecodeError` and `std.json.DecodeError` paths cleared, including
invalid/incomplete UTF-8 and malformed/partial JSON cases.

The sole remaining independent failure is:

```text
responses sse archive chunk keeps raw codex rate limit event while client output filters it
HTTP 500: assertion failed
```

This is an archive projection assertion, not an error catch or Runtime
exception-identity failure.

## Verification

The following passed:

```text
cargo test -p skiff-runtime-eval assembly_execution::projection::tests -- --nocapture
# 7 passed

cargo test -p skiff-runtime-eval --no-fail-fast
# 132 passed

cargo check --workspace
git diff --check
```

The first Relay attempt was infrastructure-only: the new worktree lacked the
Router's local `tsx` dependency. `pnpm --dir router install --offline
--frozen-lockfile` populated this worktree from the local pnpm cache, after
which the isolated run completed as recorded above.

No push, stable operation, source workaround, other-worktree modification, or
disk cleanup was performed.
