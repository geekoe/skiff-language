# Leaf task f1 (batch Slice 2a): db transaction same-package ban + remove runtime actor transaction path

## Reference chain

- Authoritative design: `doc/architecture/actor-shared-heap-design.md` v4 at
  `dc61c020d784050dfdad0392b22f0b9eb5801e87` (design §5 "事务：v1 简化决策", §12 Slice 5,
  renumbered Slice 2a by this batch because it must precede the HeapAccess slice).
- Batch interface handoff: `doc/implementation/actor-shared-heap/interfaces.md`.
- Direct parent: batch "actor-shared-heap" integration agent
  `/root/integration_actor_shared_heap`; baseline `14c06b8cb6c18b6182dfcb3842f82fa7245d2b37`
  (integration branch `integration/actor-shared-heap`, Slice 1 already merged).
- Workflow: `/Users/geek/workspace/multi-agent-development.md`.

## Contract

1. Compiler: actor-context flag in source execution-semantics analysis; reject `db:transaction`
   access in actor method and `create` profiles, including transitive local-helper reachability
   (internal `callable_effect_profiles` join, same as the pre-Slice-1 `execution_semantics/effects.rs`
   mechanism, restricted to the transaction tag). Do NOT reject `spawn` targets (detached requests)
   and do NOT reject ordinary functions. Cross-package / service-call / interface target bodies
   stay invisible (documented v1 limitation; no artifact-model/schema change).
2. Runtime: delete `ActorExecutionFrame::with_transaction_live_fields` and the actor branch in
   program_db rollback (`rollback_transaction_live_roots` always uses the None path; remove
   actor_roots collection/publication). Keep ordinary-request transaction evaluators and rollback
   (truncate + live-roots rebase) intact.
3. Tests: delete the actor-frame runtime transaction tests; add compiler rejection tests
   (direct in actor method, in `create`, local helper reachable from actor method, negative
   ordinary-helper, negative `spawn` target); add ordinary-request-only runtime transaction
   regression tests (body success, abort on body error, rollback preserving error/env roots for
   outer catch, commit error path) using the existing Db fixture without an ActorExecutionFrame.
4. Verification: focused `cargo test` for skiff-compiler-source, skiff-compiler (affected test
   binaries), skiff-runtime-eval (lib + program_db tests); `cargo fmt --check` on touched crates.

## Boundaries

- Write scope: compiler/source (execution_semantics + tests), compiler tests,
  runtime/eval (program_db rollback, actor_concurrent_continuation frame, related tests/fixtures).
- Do NOT touch router/, .github/workflows/verify.yml, artifact-model/linked-program schema.
- Ordinary single-db ops (`db require`/`insert`/`upsert`/`update`) in actor methods stay allowed.
- Do NOT change ordinary request transaction semantics; do NOT weaken fail-closed behavior.
- Do NOT merge or push; hand the completed branch/worktree/commit/evidence to the integration agent.
