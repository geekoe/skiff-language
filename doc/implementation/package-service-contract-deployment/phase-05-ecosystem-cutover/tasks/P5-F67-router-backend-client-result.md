# P5-F67 Router backend client result

- Task: `P5-F67-post-checkpoint-fanout.md` / `router-backend-client`.
- Status: `IMPLEMENTED`.
- Base candidate: `9263b9a76de66f09ca2945ba9233e6265609e739`.
- F66 correction consumed: integration `4a353bb` (cherry-picked as `6fbe766`).

## Result

- Added a Router-owned, long-lived NDJSON activation backend child. Requests are correlated by
  bounded request IDs; strict frames, unknown responses, child failures and backend errors fail all
  affected calls closed.
- The client implements only the existing `AssemblyActivationStateStore` and
  `RuntimeAssemblySnapshotLoader` boundaries. Commit forwards the exact connected and prepared
  replica sets introduced by the corrected F66 envelope.
- Added explicit `activationBackend.executablePath` plus argv configuration. The executable must be
  absolute and the object rejects unknown fields.
- Production (`profile: prod`) now requires that backend and rejects `artifactRoots` or
  `ecosystemStoreCliPath`. The old filesystem/compiler client remains an explicit non-production
  local path only.
- Router shutdown closes backend stdin and terminates a child that does not exit after EOF.

## Validation

- `git diff --check`: PASS.
- `pnpm --filter @skiff/router type-check`: SETUP ERROR because this worktree has no
  `node_modules` (`tsc: command not found`). No dependency installation or shared-worktree mutation
  was performed.

No runtime/native public bridge, stable instance, merge or push was performed.
