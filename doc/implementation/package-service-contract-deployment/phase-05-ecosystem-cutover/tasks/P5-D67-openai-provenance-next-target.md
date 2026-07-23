# P5-D67 OpenAI provenance next target

- Authority: `doc/architecture/package-service-contract-deployment.md`, exact callable provenance.
- Read-only base: F80 worktree commit `79bed78e...` (includes F76).
- Directly compile OpenAI production+overlay and trace the first remaining UnknownCallTarget after
  `JsonObject.set` through source call graph and canonical registry/transfer ownership.
- Return exact callable key, runtime-proven effects/provenance, files, and one bounded fix task. Check
  whether the target is already fixed by F78's `std.json.decode` commit, but do not merge or edit.
- No installs, commits, runtime/router/Mongo/stable, or package acceptance.

