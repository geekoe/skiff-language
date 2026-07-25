# P5-F216 bytes.toUtf8String callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

A reachable-native audit over Relay's 17 public roots and their 350 executable
dependencies shows that, after F215's event constructors, the next exact
unknown leaf in `v1Proxy` is:

```text
receiver:bytes.toUtf8String@1
```

The call occurs in Relay response handling and in llm-providers safe response
decoding. Its canonical receiver operation and Runtime implementation exist,
but exact receiver callable semantics are absent. The next later leaf is
`std.json.decode` and is intentionally outside this task.

## Required semantics

Add exact semantics for canonical `bytes.toUtf8String()`:

- validate receiver identity, zero arity, and string return type against the
  canonical operation/signature;
- successful return is a detached string and does not alias the byte receiver;
- no caller write, escape, unknown-target, same-heap requirement, or
  suspension;
- preserve the Runtime's exact typed invalid-UTF-8/decode error behavior;
- malformed receiver/arity/return/signature and non-canonical lookalikes remain
  fail-closed;
- do not generalize to other bytes receiver methods or `std.json.decode`.

## Acceptance

- Runtime tests cover valid ASCII/Unicode and invalid UTF-8 typed failure.
- Positive and negative receiver-semantics/signature tests pass.
- Focused Relay/llm-providers response-body caller shapes return detached
  string provenance without unknown, alias, escape, or same-heap effects.
- Real `chatgptPlan.responses` and Relay `v1Proxy` proceed to Available or
  record the exact next independent blocker.
- Existing compiler/Runtime tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F216-bytes-to-utf8-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. The canonical receiver signature and
Runtime UTF-8 decoder define success and typed failure behavior. Ask the
primary agent if invalid UTF-8 is not represented by an existing public typed
error.
