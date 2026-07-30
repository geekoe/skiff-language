# P5-F272 Stable field-path narrowing result

## Result

Completed.

Stable field reads now preserve the already-computed refined type in both
source typing and exact contract projection. The field expression still walks
its object first, so expression traversal and `ExpressionKey` ordering are
unchanged.

When an assignment writes a stable path, the checker removes refinements for
that path and its descendants after the right-hand side has been checked. A
write to a path prefix therefore also invalidates refinements below that
prefix.

## Coverage

Compiler source tests cover:

- a nullable field on a local nominal record;
- a nullable field whose owner and child are imported `PackageSchema` records;
- invalidation after writing the exact imported field;
- invalidation after writing a local path prefix;
- a field below an unstable call result;
- an unrelated stable field path.

The imported positive case exercises an exact non-null `PackageSchema`
argument, rather than accepting a display-name or structural fallback.

## Fresh Agine validation

A new isolated artifact store was bootstrapped at
`/tmp/skiff-f272-agine.tNAW3A/ecosystem-store`. It used this Skiff worktree,
the committed official `http-session` and `track` package fixes from
`skiff-packages-p5-f251-http-session`, and the clean Internals integration
sources.

Fresh publication succeeded for std, `http-session`, `track`, `llm-api`,
`llm-providers`, Agent, Relay, and AIHub. Agine production compilation no
longer reports either confirmed narrowing failure:

- `agent_bridge_tool_projection.skiff`: `result.error`;
- `thread_store.skiff`: `state.currentRun`.

Compilation continued to unrelated existing diagnostics, beginning with the
iterable type at `agent_bridge_llm_adapter.skiff:109` and nominal/string
projection mismatches at `agent_bridge_tool_projection.skiff:202`.

The standard Internals workflow was also attempted first; its fresh store
stopped before Agine because that fixture does not seed the
`skiff.run/http-session` pointer. The explicit isolated publication above
provided the missing official package prerequisites without using stable.
The task-owned temporary store was removed after recording the validation
result.

## Verification

- focused stable-field tests: 2 passed;
- `skiff-compiler-source` library suite: 290 passed;
- `cargo check --workspace`: passed;
- workspace rustfmt check: passed;
- `git diff --check`: passed.

Nothing was pushed and no stable instance was read or modified.
