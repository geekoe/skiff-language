# P5-F205 Legacy package root shadowing

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

Registry's canonical source test contains a normal lexical binding:

```skiff
const package = root.model.PackageArtifact { ... }
packageArtifactPut(package)
```

The compiler rejects each identifier use as:

```text
root reference package uses removed package.* spelling
```

`compiler/source/src/root_refs/mod.rs::collect_root_chain` currently treats any
`Expr::Identifier("package")` as a root-chain head, even with no following
segments and even when the name resolves to a lexical local. Resolution then
unconditionally emits `RemovedPackageSyntax`.

The language has removed the old unbound `package.<module>.<symbol>` root
syntax. That removal does not reserve the word `package` or forbid normal
lexical shadowing.

## Required implementation

1. Make legacy package-root detection lexical-resolution aware.
2. Reject only an unbound root chain using the removed package-root spelling.
3. A bare lexical identifier named `package` must remain valid.
4. A lexical local named `package` followed by normal field/member access must
   remain valid.
5. Preserve fail-closed rejection of actual old `package.<module>.<symbol>`
   syntax, including nested expression contexts.
6. Do not weaken ordinary `root.*` resolution or introduce name-based
   exceptions in Registry.

## Acceptance

- Positive tests cover bare local `package`, passing it as an argument, and
  accessing a valid field/member through it.
- Negative tests retain rejection of unbound legacy package-root references.
- Scope/shadowing tests cover nested lexical bindings.
- Existing root-reference/source tests pass.
- The Registry source test keeps its natural `package` local name and compiles.
- Real Registry package tests proceed past this validator or record the exact
  next independent blocker.
- `cargo check --workspace` and `git diff --check` pass.
- Add `P5-F205-legacy-package-root-shadowing-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow the root-reference collector and
its existing lexical resolution model. Ask the primary agent only if the
compiler lacks enough scope information to distinguish bound and unbound names
without a wider architecture change.
