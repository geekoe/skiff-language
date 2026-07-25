# P5-F245 Relay raw archive and client filtering result

## Result

F245 is complete. The focused Relay response projection suite passes 23/23.
No Relay production or test source was changed for this task.

The failing case already preserved the intended two projections:

- the archive contained the raw `codex.rate_limits` event and its payload;
- the client output omitted the private event and retained the ordinary response
  delta.

The only incorrect value was the accumulated client response text: it was
`"HiHi"` instead of `"Hi"`.

## Root cause

`archiveSseResponseChunk(bytes)` returned its caller-owned bytes. The following
catch-wrapped call to `transformResponseSseChunkUnsafe(...)` was then evaluated
twice by the Runtime. Each evaluation applied the same ordinary response delta
once, so the output bytes appeared correct while the mutable response state was
updated twice.

F249 fixed the Runtime evaluation defect in integration commit `a7d2e0c`
(`fix(runtime): evaluate executable arguments once`). The fix evaluates each
executable argument once, preserves left-to-right order, and reuses the
evaluated value for invocation.

The first clean revalidation attempts were blocked before test execution by a
separate nested Package record path defect. F252 fixed that compiler projection
defect in integration commit `ac5cfa8`
(`compiler: fix nested package record field paths`). It prevented the erroneous
path `chatgptPlan.chatgptPlan.OauthError` and restored the exact dependency
graph used for final validation.

## Focused acceptance

Command:

```bash
node scripts/skiff.mjs test \
  /Users/geek/workspace/internals-p5-f188/codex-relay/service/relay_responses_projection.test.skiff \
  --artifact-root /private/tmp/p5-f251-existing.50R3Sj/store
```

Result:

```text
test result: ok. 23 passed; 0 failed
```

The focused suite covers event-named and data-only rate-limit frames, mixed
private and ordinary frames, raw archive preservation, client filtering,
ordinary response-state accumulation, split UTF-8 input, split SSE delimiters,
and incomplete or invalid UTF-8 tails.

The same exact graph produced Relay artifact
`117af71a55a29338fd63d0330e38dd4166cbee8e8c10e09043dd4690c56f8f15`,
ABI `a219ebd81a60894c766cd81092dbc6cac13c6f8d1510ddc62aeca6b803cf3d58`,
with 17/17 public APIs available.

## Complete Relay suite

Command:

```bash
node /Users/geek/workspace/skiff-phase-05-integration/scripts/skiff.mjs test \
  codex-relay/service \
  --artifact-root /private/tmp/p5-f251-existing.50R3Sj/store
```

The complete suite retained all 23 F245 projection passes and reported 25
independent failures:

- 16 business-behavior assertion failures across admin authentication, legacy
  ChatGPT migration, upstream selection/recovery, and two Relay route cases;
- 4 package response-health cases failed with
  `emit used outside a stream output context`;
- 5 unary Responses cases reached the `std.http.client.sse` test double, but
  request-subset matching received a heap handle instead of a materialized HTTP
  request.

These failures do not involve raw archive preservation, private rate-limit
filtering, split-chunk buffering, or duplicate response-state accumulation.
They are follow-up Runtime/test-harness or Relay behavior work and were not
worked around in F245.

## Repository state

- Relay production changes: none.
- Relay test changes: none.
- Skiff prerequisite: F249 (`a7d2e0c`).
- Compiler prerequisite: F252 (`ac5cfa8`).
- Stable instance: not used.
- Push: not performed.
