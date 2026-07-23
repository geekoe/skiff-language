# P5-D65 Production callable facts handoff audit

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical package artifact
  callable facts and test overlay boundary.
- Candidate: current Skiff/skiff-packages integrations; I70B shows all focused semantics probes PASS
  while all four real package fixtures still lose facts.
- Read-only closure: trace representative exact paths end to end:
  - `aliyunoss.createUploadUrl`
  - `track.record`
  - `openai` first test production call
  - `http-session` first test production call
  from source target/effect analysis → FileIR/package-local callable semantic facts → PackageArtifact
  record → typed test overlay source graph → exported `testCases.case0` boundary projection.
- At each hop record exact callable identity/key and facts, identify where each fact is dropped,
  duplicated, conservatively recomputed, or keyed differently. Include artifact serialization and
  owner-local private/public identities; explain why focused probes miss the real gap.
- Return one bounded fix wave with exact file ownership and a cheapest combined probe that covers all
  four without four full runtime runs. No edits, installs, commits, stable, or full gate.

