# P5-D72 Registry native chain removal audit

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at commit `335957b`.
- Read-only scope: enumerate every production/test/doc/Cargo surface introduced for the erroneous
  registry-as-native/compiler-platform model: trusted-registry contract crate, native signatures/
  dispatch/context/scopes, compiler authority/descriptor/synthetic declarations, special package id,
  capability projection, Rust PlatformDbTrustedRegistry and skiff root registry placeholder.
- Separate pure removals from shared types/tests needed by unrelated package/service/deployment
  behavior. Return exact files/symbols/dependency order and focused zero-surface/ordinary-package
  regression probes.
- No edits, installs, commits, stable, or full gate.

