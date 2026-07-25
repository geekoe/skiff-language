# P5-F225 Map.get callable semantics

## Context

After F224, llm-api materialization first becomes conservative at canonical
`receiver:Map.get@1`. Runtime returns an optional map value; heap-backed values
retain reachability from the receiver.

## Required implementation

- Validate exact generic Map receiver, key argument, optional value return, and
  canonical operation identity.
- Model return as receiver-reachable alias with same-heap requirement, no
  mutation, escape, unknown target, or suspension.
- Preserve missing-key null/optional behavior.
- Prove contextual transfer: caller-owned Map retains alias/same-heap, while a
  Fresh local accumulator maps the result to Fresh and discharges same-heap.
- Keep malformed signatures and lookalikes fail-closed.

## Acceptance

- Runtime identity tests cover scalar, nested heap value, and missing key.
- Compiler positive/negative/context tests pass.
- Real materializeCompletedResult proceeds or records the next exact blocker.
- Existing tests, workspace check, diff check, result document, and commit.
- No push or stable operations.
