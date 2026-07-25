# P5-F253 Cross-file publication effect closure result

## Outcome

The reported cross-file closure gap was not present in the compiler at the
integration baseline.

Source callable analysis already builds one Package-wide definition index and
one Package-wide SCC graph before lowering. Exact `root.<module>.<callable>`
calls therefore participate in the same fixed point regardless of their source
file. Lowering subsequently rewrites those exact cross-file targets to
`publicationExecutable`; it does not own a second or weaker effect analysis.
Adding another FileIR-level effect analyzer would duplicate the canonical
source facts and create two competing effect owners.

The investigation added full regression coverage for the existing canonical
path instead:

- direct and transitive calls across three files;
- mutual recursion across files;
- caller-alias return provenance;
- caller-reachable writes and heap-identity requirements;
- database escape and suspension;
- detached throw provenance;
- missing module and ambiguous declaration failures.

Missing and ambiguous publication targets fail before artifacts can be
projected. Package dependencies and ServiceContract operations continue to use
their distinct resolved target kinds.

## Real AIHub

AIHub was rebuilt from the F251 probe source with this worktree's compiler and
an isolated copy of `/tmp/p5-f251-existing.50R3Sj/store`. The published
operations remained unavailable, but tracing the source fixed point found no
missing or unknown resolved call target.

The next exact HTTP blocker is:

```text
internal.aihub_service.applyProviderOptions
  -> std.json.merge
  -> native callable semantics missing
  -> unknownCallTarget
  -> handleAihubHttp
```

`applyProviderOptions` is in the same file as `handleAihubHttp`; the failure is
not caused by `publicationExecutable`. The conservative
`returnsCallerAlias` reason is propagated from that unknown native call.

The F251 WebSocket source with nominal `AihubSocketContext` still has a
separate, earlier imported-generic descriptor blocker for
`std.websocket.WebSocketIngressEvent<AihubSocketContext>`. The `<string>`
diagnostic probe reaches the same `std.json.merge` effect blocker. Neither
AIHub source nor its wrapper was changed here.

## Validation

- focused cross-file callable tests: 2 passed;
- complete compiler source suite: 281 passed;
- cross-module publication lowering test: 1 passed;
- real AIHub isolated publish: passed and exposed `std.json.merge`;
- workspace check: passed.

No push, stable-instance operation or disk cleanup was performed.
