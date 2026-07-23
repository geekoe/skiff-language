# P5-F93 JsonObject receiver lowering

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact typed FileIR call
  targets.
- Candidate: current Skiff integration; F92 advances agent to
  `drain_checkpoint_store.skiff` JsonObject.set receiver lowering failure.
- Worktree: create `skiff-p5-f93-json-object-receiver-lowering`.
- Write owner: compiler source/lowering receiver type→builtin op resolution and focused tests.
- Required outcome: exact nominal JsonObject receivers, including source aliases/locals with explicit
  JsonObject targets, resolve `set` to canonical `receiver:JsonObject.set@1`. Preserve its W/I
  semantics and contextual fresh discharge. Do not route unknown objects by method name.
- Fail closed: Map/record/unknown/other nominal receiver, wrong arity/value type, forged method/key.
- Validation: drain-checkpoint-shaped source fixture plus fresh std→llm-api→agent compile-only. Stop
  at next independent diagnostic. No Internals/package edits, stable, merge, push, or full workflow.
- Deliver one commit/evidence.

