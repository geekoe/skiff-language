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
