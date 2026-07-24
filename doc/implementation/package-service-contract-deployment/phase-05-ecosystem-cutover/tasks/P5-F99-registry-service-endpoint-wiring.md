# P5-F99 Registry service endpoint wiring

- Authority: `doc/architecture/package-service-contract-deployment.md` §13 at `335957b`.
- Candidate: skiff-packages current integration after F96/F97.
- Worktree: create `skiff-packages-p5-f99-registry-endpoints`.
- Write owner: Registry shared API export closure, service operation implementations/bindings and
  focused service tests.
- Required outcome: export every contract-visible nominal type, bind all 20 contract operations to
  ordinary Registry service functions, delegate four Put/Read families to immutable_store and four
  pointer families to pointer_store, and map internal outcomes to the strict typed error union.
- Preserve exact typed schemas and DB requirement; no activation/native/compiler authority/client
  package/raw Json/common artifact union. Do not duplicate storage/CAS algorithms.
- Validation: real contract/service/deployment authoring, all operation bindings exact, focused
  positive and invalid/notFound/conflict/cas/transaction tests. No Skiff edits, stable, merge, push or
  full gate. Deliver one commit/evidence.

