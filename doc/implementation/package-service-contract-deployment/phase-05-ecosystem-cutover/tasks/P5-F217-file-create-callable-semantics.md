# P5-F217 file.create callable semantics

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

After F215, three Relay public chains share the same first unknown leaf:

```text
v1Proxy
relayProxy.responsesCompleted
relayProxy.responsesCompletedResult
  -> interactions.startInteraction
  -> archiveRequestFile / archiveResponseBytes
  -> std.file.create
```

The canonical native signatures exist:

```text
std.file.create(bytes, CreateOptions?) -> ImmutableFile
std.file.createFromStream(Stream<bytes>, CreateOptions?) -> ImmutableFile
```

Both use File capability context and asynchronously create a file. The stream
variant consumes typed byte items into staging storage and commits on natural
end, cleaning up and cancelling on decode, source, write, or commit failure.
Exact compiler callable semantics are absent for both reachable bindings.

## Required semantics

Add exact semantics for canonical `std.file.create` and
`std.file.createFromStream` only:

- validate each exact binding identity, its bytes or Stream<bytes> parameter,
  nullable CreateOptions parameter, and ImmutableFile return type against the
  native signature;
- required context is File;
- `maySuspend=true`;
- successful return is a newly created detached ImmutableFile value;
- no caller alias, caller write, caller escape, unknown-target, or same-heap
  requirement;
- preserve exact typed File capability, stream source/decode, cancellation, and
  cleanup errors without caller-alias provenance;
- malformed signature/arity/types/context/route and non-canonical lookalikes
  remain fail-closed;
- do not generalize to createText or any read/other File operation.

## Acceptance

- Artifact/compiler tests cover exact success semantics and malformed/
  lookalike rejection.
- Runtime route/context/signature parity is validated.
- File capability success and typed error tests prove Fresh return and no
  caller alias/escape for both bindings.
- Existing createFromStream natural-end, decode, source, write, commit,
  cleanup, and cancellation tests remain green.
- Focused archiveRequestFile/archiveResponseBytes caller shapes have only the
  real suspension effect.
- Real Relay `v1Proxy`, `relayProxy.responsesCompleted`, and
  `responsesCompletedResult` proceed to Available or record the exact next
  independent blockers.
- Existing compiler/Runtime tests, `cargo check --workspace`, and
  `git diff --check` pass.
- Add `P5-F217-file-create-callable-semantics-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. The two canonical native signatures and
File capability Runtime routes define exact behavior. Ask the primary agent if
the ImmutableFile result retains caller-owned bytes, stream items, or
capability storage contrary to the detached behavior described above.
