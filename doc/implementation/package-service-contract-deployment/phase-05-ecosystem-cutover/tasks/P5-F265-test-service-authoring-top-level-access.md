# P5-F265 Test service authoring and top-level dependency access

## Authority

`doc/reference/testing.md`

## Required implementation

- Add `service.yml: kind: test` authoring.
- Test services produce ordinary PackageArtifact, ServiceContract, Deployment
  and RuntimeAssembly formats.
- Permit dependency `access: topLevel` only for `kind: test`.
- In topLevel mode, resolve only the exact dependency implementation
  source-module/top-level symbol index and completely ignore its `api.yml`.
- Syntax is `alias/source.module.name`; `root.*` remains the test service.
- Public and topLevel modes never fall back to each other.
- Bind an exact dependency implementation build/ABI; access is non-transitive.
- Ordinary publish/deploy/watch commands reject `kind: test`; `skiff test`
  accepts it.

## Acceptance

- Positive ordinary artifact/link/runtime test service chain.
- Public/topLevel same-name collision proves the selected mode is exclusive.
- Non-test topLevel, transitive private access, missing source symbol, stale
  build/ABI and publish/deploy negatives.
- Existing production package/service authoring remains unchanged.
- Workspace check, result and commit.
