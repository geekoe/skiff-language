# P5-F97 Registry service storage implementation

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Predecessor: skiff-packages Registry contract checkpoint `1280205a`.
- Parallel skiff-packages file owners:
  - `immutable-records`: `registry/immutable_store.skiff` and focused tests only. Implement typed
    put/read for PackageArtifact, ServiceContract, ServiceDeployment and RuntimeAssembly using
    ordinary `std.db`; recompute/validate canonical identity fields, insert-if-absent, same-identity
    same-content idempotency and different-content immutableConflict.
  - `pointers`: `registry/pointer_store.skiff` and focused tests only. Implement four typed current/
    history stores and transactional CAS with exact expected/candidate/key/target validation,
    monotonic sequence, bounded ascending history pagination and atomic current+history update.
- Shared model/operation schemas are frozen by F96; do not edit contract.yml/generated boundary
  schemas or invent common artifact unions/raw Json/native operations.
- Database requirement is ordinary `registry-store`; code sees no URL/provider config. Collections
  must be isolated from Router activation state/audit.
- Validation: shard-focused std.db tests and transaction failure negatives. No endpoint wrapper/
  operation binding yet, Skiff edits, stable, merge, push, or full gate. Deliver one commit per shard.

