# P5-F60 Host legacy module closure

- Authority: `doc/architecture/package-service-contract-deployment.md`, whole-assembly runtime
  admission and terminal legacy-removal requirements.
- Predecessor: I55C FAIL and D58 closure audit at Skiff integration commit `bf9c42ed770fe107440ba91acf16a0f0aa24899c`.
- Repository/worktree: create `skiff-p5-f60-host-closure` from
  `/Users/geek/workspace/skiff-phase-05-integration`.
- Write owner: `runtime/host` and the directly corresponding `runtime/driver` legacy exports.
- Required outcome: migrate still-valid request heap/config/service-DB/spawn behavior to canonical
  assembly/activation owners, then delete the complete ArtifactGraph/cache/pointer/program-loader,
  RuntimeServiceConfig/ServiceRuntimeContext/legacy route/state closure and its legacy tests. Keep
  assembly admission, active assembly context, and runtime assembly request paths.
- Prohibited: restoring aliases/shims, compatibility loaders, filesystem fallback, or editing the
  trusted-registry checkpoint surface owned by F61.
- Validation: reverse-search the D58 symbol set; `cargo check -p skiff-runtime-host --all-targets`;
  `cargo check -p skiff-runtime-driver --all-targets`; focused canonical assembly tests. Avoid the
  full gate.
- Deliver one commit and evidence; do not merge or push.

