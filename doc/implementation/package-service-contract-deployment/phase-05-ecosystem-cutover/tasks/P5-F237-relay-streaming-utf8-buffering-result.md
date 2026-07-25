# P5-F237 Relay streaming UTF-8 buffering result

Status: Relay byte-buffering implementation complete. Canonical execution now passes every
positive UTF-8 framing case; error-path verification is blocked by the independently tracked
canonical std-error catch identity defect in P5-F244.

## Implementation

- Relay now concatenates retained raw bytes with each transport chunk before protocol parsing.
- SSE boundaries are found in the byte representation before any UTF-8 conversion.
- Only complete SSE protocol units are decoded.
- A trailing partial unit remains in `UpstreamStreamState.ssePendingBytes` as raw bytes.
- Complete units are applied and sanitized in their original order.
- Invalid completed UTF-8 and an incomplete sequence at EOF remain retained and produce no
  forwarded output, preserving the existing fail-closed behavior.
- `bytes.toUtf8String()` remains strict. The implementation does not replace invalid input or
  special-case fixture bytes.

Internals commit:

```text
870346f fix(relay): buffer raw SSE bytes across chunks
```

## Coverage

The Relay file suite now covers:

- a complete multi-byte UTF-8 protocol unit;
- one UTF-8 code point split across transport chunks;
- multiple complete events in one chunk;
- an SSE delimiter split across chunks;
- invalid UTF-8 in a completed unit;
- EOF with an incomplete UTF-8 sequence.

Existing response projection tests continue to cover CRLF framing, output-delta aggregation,
completed-output fallback, sanitization, rate-limit filtering, and multiple production event
types.

## Canonical prerequisites found

The first focused canonical run compiled the new parser and tests, then stopped before executing
`testCases.case2`:

```text
invalid canonical fixture:
test callable pkg-callable:agine.ai/codex-relay:testCases.case2
cannot cross the canonical test boundary: [UnknownCallTarget]
```

The exact missing target is `receiver:bytes.toHex@1`. The language signature and Runtime handler
already exist, but the callable-semantics registry does not yet describe its Fresh, pure result.
P5-F238 owned that independent compiler prerequisite and is now complete. Relay cannot correctly find a raw-byte SSE
delimiter using the existing language API without such a byte-safe representation; reverting to
per-transport-chunk UTF-8 decoding would recreate the original defect.

The focused suite then exposed a second independent canonical Runtime defect. The source catches
`std.bytes.DecodeError` and `std.json.DecodeError`, but canonical assembly projection has no legacy
`PackageUnit` entries. Standard error address recognition still consulted only that legacy list,
so the linked catch type retained only its address identity while native errors carried the
builtin identity. P5-F244 owns that Runtime identity bridge.

## Verification

- `git diff --check`: passed for the Internals commits.
- Focused response-projection file: 19 passed, 5 failed.
- All new positive cases passed: complete UTF-8, split code point, multiple events and split
  delimiter.
- The two new invalid/incomplete UTF-8 cases reached the intended strict decode but their catches
  were bypassed by P5-F244.
- The other three failures are existing archive assertion and JSON catch-path failures; the two
  JSON failures share P5-F244's canonical catch defect.
- Relay full service crossed FileIR activation after P5-F243 and executed the suite. The remaining
  failures are runtime/test behavior, not compilation or artifact activation.
- No stable instance was operated, nothing was pushed, and no worktree, cache, or shared disk state
  was cleaned.
