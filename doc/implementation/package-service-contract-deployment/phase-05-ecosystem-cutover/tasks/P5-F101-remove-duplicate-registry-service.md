# P5-F101 Remove duplicate Registry service

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Candidate: current Internals integration; D76 proved the old owner has no production callers or
  stable watch entry.
- Worktree: create `internals-p5-f101-remove-old-registry`.
- Write owner: delete `skiff-platform/package-registry/**`, remove its client service-generation
  entry/regenerate metadata, and remove/update Internals docs that claim the old upload/build/catalog
  routes are `skiff.run/registry`.
- Required outcome: Internals contains no production owner/reference for service id
  `skiff.run/registry`; canonical owner is only skiff-packages. Do not rename the old mixed service
  or preserve compatibility/dual-write.
- Preserve product requirements for source upload/catalog/build only as a clearly nonimplemented
  backlog under distinct future service names; no code/collections/routes remain.
- Validation: global service-id/caller/generated/watch reverse search and affected client generation/
  type-check. No stable, merge, push, build/dev/start or full gate. Deliver one commit/evidence.

