# P5-F80 JsonObject.set callable semantics

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact callable effects.
- Candidate/base: F76 checkpoint `2b474082...`; D66 traced OpenAI's first remaining target to
  `receiver:JsonObject.set@1`.
- Worktree: create a new Skiff worktree/branch at exact F76 commit.
- Write owner: artifact-model canonical receiver callable semantics for JsonObject.set and focused
  compiler/source transfer tests. Do not edit F78's other transfer implementation.
- Required fact: JsonObject.set mutates the receiver and embeds the value in that heap graph,
  returns constant null, does not suspend/invoke unknown/return or throw caller alias. Caller-owned
  receiver retains W+I; function-local fresh receiver is contextually discharged by F76.
- Validation: artifact-model matrix/mutation negatives, focused compiler wrapper with fresh vs
  caller-owned JsonObject, and direct OpenAI production+overlay compile-only case0 probe.
- Deliver commit(s) atop F76 and evidence; no package/runtime/artifact boundary edits, stable, merge,
  push, compatibility, or full gate.

