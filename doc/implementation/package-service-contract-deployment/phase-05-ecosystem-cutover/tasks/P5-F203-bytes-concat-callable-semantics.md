# P5-F203 bytes.concat callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

The official OpenAI package builds, but canonical package-test admission rejects
`testCases.case1` with `UnknownCallTarget`. The failure is already present in
the production operation:

```text
imageEdit
  -> prepareImageEditRequest
  -> multipartBody
  -> bytes.concat(chunks)
```

`imageGenerate` is fully analyzed. `imageEdit` currently reports
`invokesUnknownTarget=true`, `requiresSameHeapIdentity=true`, and unknown
return provenance.

`core.bytes.concat` already has a canonical native signature in
`artifact-model/src/native_signature.rs` and a Runtime native handler. The
compiler-owned std native callable semantics registry contains
`bytes.fromBase64` and `bytes.fromUtf8`, but omits `bytes.concat`.

## Required semantics

Add exact callable semantics for the existing canonical `bytes.concat`
signature:

- validate the canonical receiver/function identity and exact argument shape;
- result provenance is `Fresh`;
- no caller alias, write, escape, unknown-call, or same-heap requirement;
- `may_suspend=false`;
- do not generalize semantics to other bytes functions;
- malformed receiver, arity, argument type, or non-canonical lookalike remains
  fail-closed.

Do not modify the Runtime handler unless a real signature parity defect is
found. Do not introduce handwritten service/package exceptions.

## Acceptance

- Native semantics registry positive and negative tests pass.
- A focused source callable-effects test covers the OpenAI multipart shape.
- The real official OpenAI package's `imageEdit` is fully analyzed without
  `UnknownCallTarget` or same-heap identity.
- Real OpenAI package tests pass through the isolated package-test Runtime, or
  the exact next independent blocker is recorded.
- `cargo check --workspace` and `git diff --check` pass.
- Add `P5-F203-bytes-concat-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

This task is the immediate authority. Follow referenced implementation only as
needed. Ask the primary agent only if the actual canonical signature cannot
support the semantics above without a design change.
