# P5-F264 Agent public schema closure

## Context

After F259 and F262, Agent clears existential-interface and Package nominal
JSON codec failures. Publication now rejects:

```text
boundary named child model.AgentToolResultStatus is not explicitly public in api.yml
```

The Agent public API exposes records/unions that reference this type, but the
Package schema closure requires every boundary-named child to be explicitly
published.

## Required implementation

- Compute the complete reachable named-type closure of Agent's public
  callables, constants, interfaces and types.
- Add every intentional boundary-named child to `packages/agent/api.yml`,
  beginning with `AgentToolResultStatus`.
- Remove no internal-only type merely to silence closure validation; expose only
  types actually reachable from the public boundary.
- Add a repository test that compares generated reachable closure with explicit
  API declarations and reports missing public paths.

## Acceptance

- Agent PackageArtifact and Service/Package schema projection publish from the
  fresh exact graph.
- No reachable named child is implicit or missing.
- Existing public paths remain stable and duplicates are rejected.
- Agent tests and downstream Agine build proceed to the next independent gate.
- Internals checks, result and commit.
- No push, stable operation or disk cleanup.

## Result

- Internals implementation: `46fa76e` on
  `codex/p5-f264-agent-public-schema-closure`.
- Agent now explicitly publishes both reachable children found by the fresh
  projection:
  - `model.AgentToolResultStatus`;
  - `context.ContextFactSourceKind`.
- The repository audit computes the reachable named source closure, rejects
  duplicate API targets, reports missing source paths, and can compare a
  generated Package schema index with the explicit `api.yml` paths. Its four
  direct tests pass. The real Agent publication reports 140 reachable source
  types and 136 generated serializable schema paths; all generated paths are
  explicit.
- Fresh canonical std, `llm-api`, and Agent publication succeeded. The
  downstream retained-store revalidation published Agent build
  `38fa645a8e2dd4d00c69d78e03ed1e7087e193099fbf9fe2e72ef98010250eba`
  with local ABI
  `149653824be92962d895de01c2273ffb5789b1ffdcd437e4cd9e91aa85f9ab9d`.
- Agine consequently crossed the Agent schema gate. Its next independent gate
  is the missing official `skiff.run/http-session@1.0.0` pointer; fresh
  publication of that package exposes
  `boundary named child session.HttpSessionSource is not explicitly public in api.yml`.
  This is an official-package API closure defect, not an Agent defect.
- Agent's complete runtime test assembly proceeds past package publication but
  remains blocked before execution by existing test-source typing failures:
  nullable values compared with `null` after narrowing and object literals
  without explicit target types. Those failures are independent of the public
  schema closure.
- The branch is based on Internals `main` at `5861c13`. Exact carried
  prerequisites are F188 Agent/LLM commits `34ba344`, `3fd6659`, `78f0de1`,
  `5a73444`, `c3773de`, `47d31bc`, `6153ea4`, and `767cd88`; the Agent database
  state declaration from `63b78c8`; and F247's chain `3f5f0dd`, `16edba5`,
  `e7d9940`, `c03a7e3`, and `0b290d3`. No unrelated service-state files from
  `63b78c8` were carried.
- No push, stable operation, or disk cleanup was performed.
