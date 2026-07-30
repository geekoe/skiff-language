# P5-F100 Runtime service DB provisioning

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Predecessor: F98 activation control wire at current Skiff integration.
- Worktree: create `skiff-p5-f100-runtime-service-db`.
- Write owner: Runtime assembly staging/admission, active activation-owned DB capability source,
  Host request adapter and focused tests. Do not edit Router emission.
- Required outcome: stage Router-supplied serviceDb only for the candidate generation; for each
  activation with exact Database StateBinding and closed runtime DB metadata, call existing
  DbProviderSource to build an activation-owned source. Commit/recovery preserves it; abort/retire
  drops it. Request context obtains exact activation/request-scoped DB context; remove unconditional
  unavailable. Provider config is never read from runtime file/env/artifact.
- Fail closed: DB binding without provider, provider without valid binding/metadata, duplicate/
  unsupported state binding, wrong activation/generation, retired source reuse. Non-DB activation
  remains unavailable and does not construct a provider.
- Validation: own/dependency namespace isolation, generation A/B isolation, commit replay, abort/
  retire and missing negatives; focused Host/runtime tests. No Router config/server, package,
  stable, merge, push or full gate. Deliver one commit/evidence.

