# P5-I94 Registry privilege removal acceptance

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Candidate: current Skiff integration after F94.
- Read-only verdict: prove no production/test/Cargo/compiler/runtime/deployment special privilege
  remains for registry: trusted contract/native specs/context/capability, compiler authority/
  descriptor/synthetic sources/reserved id, Rust Platform DB adapter, Rust activation envelope and
  Skiff-root registry source.
- Preserve and probe ordinary std, package/service authoring, generic runtime requirements, four
  artifact types, local filesystem store and Router serviceDb configuration.
- Run affected checks/tests once with low-disk settings and exact reverse searches. Do not edit,
  install, commit, merge, push, stable, full repository gate or Router TS subprocess removal (separate
  migration).

