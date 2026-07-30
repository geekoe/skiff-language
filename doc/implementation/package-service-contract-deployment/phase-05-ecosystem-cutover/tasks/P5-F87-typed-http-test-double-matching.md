# P5-F87 Typed HTTP test-double matching

- Authority: `doc/architecture/package-service-contract-deployment.md`, canonical package test
  execution through typed native boundaries.
- Candidate: current Skiff integration; D68 proved Aliyun OSS fixtures are correct.
- Worktree: create `skiff-p5-f87-typed-http-double` from current integration.
- Write owner: Runtime Host test-effect-double HTTP request decode/matcher and focused tests.
- Required outcome: consume the existing native argument plan to decode expected
  `HttpClientRequest`/`HttpHeader`/bytes into the same typed heap representation as actual values,
  then perform exact/subset fixture matching as designed. Do not broadly treat Object and Map as
  interchangeable or weaken real mismatch detection.
- Validation: typed request/header/body positive, allowed extra actual fields if fixture subset
  semantics require them, and method/url/header/body/signature mismatch negatives; focused host
  tests plus Aliyun OSS three HTTP cases if practical.
- No package edits, production HTTP behavior changes, stable, merge, push, or full gate.

