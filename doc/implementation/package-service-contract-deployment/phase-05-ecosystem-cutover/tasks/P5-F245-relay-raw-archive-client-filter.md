# P5-F245 Relay raw archive and client filtering

## Context

After F244, `relay_responses_projection.test.skiff` passes 22/23. The sole
failure is:

```text
responses sse archive chunk keeps raw codex rate limit event while client output filters it
```

The intended invariant is two distinct projections of the same upstream bytes:
the archive retains the raw `codex.rate_limits` event and payload, while the
client stream removes that private event and still forwards ordinary response
events.

## Required investigation and implementation

- Identify the exact failing assertion and actual archive/client byte content.
- Trace both projections through the production functions, including any
  aliasing or mutation of the shared input bytes.
- Restore the invariant without test-specific strings:
  - archive bytes remain raw and complete;
  - client filtering removes only private Codex rate-limit frames;
  - ordinary frames and response state remain correct.
- Preserve split-chunk UTF-8 and SSE buffering behavior from F237.

## Acceptance

- Focused tests cover event-named and data-only rate-limit frames, mixed
  private/ordinary frames, input immutability, and split chunks.
- Relay response projection file suite passes 23/23.
- Run the complete Relay service suite and record the next independent
  failures.
- Internals checks, result and commit.
- No push, stable operation or disk cleanup.
