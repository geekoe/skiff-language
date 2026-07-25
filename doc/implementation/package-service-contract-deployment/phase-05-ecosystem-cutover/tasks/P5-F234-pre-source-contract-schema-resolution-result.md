# P5-F234 Pre-source contract schema resolution result

## Outcome

Package compilation now resolves canonical `ResolvedPackageSchema` bundles
before source compilation when a declared ServiceContract requires Package
types.

The resolver is owned by the shared compiler pipeline, so package authoring and
canonical package tests cannot diverge. It:

- accepts exact direct manifest Package dependencies and compiler-owned `std`;
- preserves manifest aliases and exact artifact version, build and local ABI;
- resolves the complete verified index and reachable record closure through
  `CanonicalArtifactStore::resolve_package_artifact_schema`;
- rejects duplicate owners, exact artifacts and resolved owner bindings;
- leaves undeclared non-std owners absent so contract validation reports
  `MissingPackageSchema`;
- never selects latest by package ID, infers an indirect dependency, or inlines
  schema records into a ServiceContract.

The resolved bundle is supplied to both ServiceContract validation and normal
source dependency analysis. Post-source projection reuses the same bundle and
retains the existing exact requirement/artifact validation.

Validated ServiceContract aliases are also admitted by package import
validation. This closes the next source boundary reached by real AIHub without
allowing arbitrary undeclared imports.

## Tests

Passed:

- focused direct/std/undeclared-transitive/manifest-alias and duplicate
  owner/artifact/std tests: 2 passed;
- compiler-input contract dependency tests: 5 passed;
- deployment storage/schema tamper, owner, closure and identity tests:
  52 passed;
- compiler-source tests: 271 passed;
- test-runner tests: 29 passed, 2 ignored;
- `cargo check --workspace`;
- `git diff --check`.

The full compiler library run has one pre-existing golden failure:
`official_std_authoring_and_record_writer_are_fixed_and_deterministic` expects
build `4cf082...`, while this integration baseline produces `d1a08e...`. The
other 17 compiler library tests, including all new tests, passed.

## Real Relay -> AIHub -> Agine

No stable instance was used. Relay was republished from
`/Users/geek/workspace/internals-p5-f188/codex-relay/service` with this
worktree compiler into the retained isolated canonical store
`/tmp/skiff-f227-relay.iB6dOG`:

```text
skiff-package-build-v4:sha256:6cc37cd4074fa0c0a6ad7a183fdb6157444da83bc084de4d286147349edef3cf
```

AIHub then crossed both the original `MissingPackageSchema` gate for Relay's
five std-owned contract types and the service-alias import gate. It reached the
next independent expression type-model gate, including nominally identical
`Array<std.http.HttpHeader>` comparisons and unresolved llmApi union facts.
Consequently the f828/f188 integration baseline does not yet publish AIHub, and
Agine cannot be run beyond it in this branch.

The integration checkout's fresh isolated-graph helper is independently
blocked even earlier because its `llm-providers/package.yml` lacks the database
state requirement used by source. Neither baseline blocker was weakened or
worked around.

There was no push, stable operation, schema inlining, version guessing or disk
cleanup.
