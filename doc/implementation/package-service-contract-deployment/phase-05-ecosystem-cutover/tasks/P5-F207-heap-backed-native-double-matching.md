# P5-F207 Heap-backed native double matching

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F203, the official OpenAI package's generate test passes and its edit test
enters the Runtime. The edit test reaches the committed HTTP native double but
matching fails:

```text
expected structured HttpClientRequest subset (multipart bytes)
got RuntimeValue::Heap(HeapHandle { ... })
```

The assembly native double dispatch is working. The matcher compares the raw
`RuntimeValue` directly. Multipart edit requests contain heap-backed
record/array/bytes values, so the matcher must materialize them through the
current request heap before applying the committed structured expectation.

## Required implementation

1. At native invocation/test-effect matching, project the actual value through
   the current request heap into the canonical deterministic wire value used by
   committed expectations.
2. Preserve structured subset matching semantics.
3. Preserve exact bytes representation, including canonical Base64 encoding.
4. Support nested heap-backed records, arrays, optionals/unions, and bytes that
   are reachable from the request value.
5. Reject invalid handles, wrong heaps, cycles, malformed values, type-shape
   mismatches, and non-canonical bytes fail-closed with precise errors.
6. Do not weaken matching to debug strings, opaque handle identity, or
   same-process heap identity.
7. Do not alter production native dispatch outside test mode.

## Acceptance

- Focused tests cover heap-backed multipart request matching and nested values.
- Negative tests cover wrong heap/invalid handle, shape mismatch, byte mismatch,
  and prove a mismatch does not invoke the real native handler.
- Existing non-heap and generate-request native double tests remain green.
- Real official OpenAI generate and edit package tests pass.
- Relevant Runtime/test-runner tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F207-heap-backed-native-double-matching-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow only the test-effect matcher,
request heap, and canonical wire projection paths it directly uses. Ask the
primary agent if no existing canonical projection can represent the committed
HTTP expectation without changing its artifact format.
