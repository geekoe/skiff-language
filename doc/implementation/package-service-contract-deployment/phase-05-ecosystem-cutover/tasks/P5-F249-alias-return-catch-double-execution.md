# P5-F249 Alias return followed by catch double execution

## Context

P5-F245 proved Relay's filtering logic is correct, but a Runtime execution bug
duplicates the next effectful call.

The trigger is:

1. a callee returns its `bytes` parameter directly;
2. the caller uses a receiver method on that caller-heap alias;
3. a following `catch(effectfulCall(state))` executes the effectful call twice.

Instrumentation shows the unsafe transform entry and frame application both
run twice. The state becomes `HiHi`, while catch retains only one returned
client output.

## Required implementation

- Build a minimal compiler-to-assembly-to-Runtime fixture:
  `alias(bytes) -> bytes { return value }`;
  `alias(...).toUtf8String()`;
  `catch(effectfulCall(state))`.
- Identify the duplicated interpreter/evaluation path and ensure every source
  expression is evaluated exactly once.
- Preserve caller alias identity and catch materialization semantics.
- Audit direct alias returns, chained receiver calls, optional/union narrowing
  and catch success/error paths.
- Do not rewrite Relay to avoid alias returns or remove catch.

## Acceptance

- The minimal effect counter increments exactly once.
- Positive matrix covers direct return, receiver chain, catch success and
  catch error; no-return-alias control remains unchanged.
- Existing Runtime eval/assembly/catch tests pass.
- Relay response projection suite passes 23/23 with its original production
  behavior.
- Workspace check, diff check, result and commit.
- No push, stable operation or disk cleanup.
