# P5-F204 Module constant callable summary

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

The latest canonical Relay artifact isolates its remaining boundary rejection to
this source shape:

```skiff
const UPSTREAM_KIND_API_KEY = "apiKey"

pub fn upstreamKindApiKey() -> string {
  return UPSTREAM_KIND_API_KEY
}
```

Calling `upstreamKindApiKey()` while building a fresh record causes the public
operation to report `UnknownCallTarget` and `returnsCallerAlias`. Replacing the
call with the literal `"apiKey"` makes the operation Available. An empty
database loop and the surrounding fresh-record construction are independently
Available.

The defect belongs to compiler callable summary/provenance lookup for a
zero-argument executable returning a module-level constant. It must not be
worked around in Relay source.

## Required implementation

1. Add a minimal compiler fixture with a module-level string constant, a
   zero-argument function returning it, and a caller placing that result in a
   fresh record.
2. Make the callable summary and lookup preserve exact `Constant` provenance
   through the function call.
3. The call must not set unknown-target, caller-alias, same-heap, write, or
   escape flags, and must not become suspending.
4. Preserve fail-closed behavior for unresolved globals, unsupported
   initializers, cyclic constants, unknown callees, and non-constant returned
   values.
5. Do not add Relay/package-name special cases or treat arbitrary zero-argument
   functions as constant.

## Acceptance

- Focused positive and negative callable-effects tests pass.
- Existing compiler source callable-effects tests pass.
- The real Relay `upstreamKindApiKey()` summary is constant and the affected
  public operation no longer reports unknown target or caller alias.
- Canonical Relay receipt proceeds to Available or records the exact next
  independent blocker.
- `cargo check --workspace` and `git diff --check` pass.
- Add `P5-F204-module-constant-callable-summary-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as the immediate authority. Follow only directly referenced
compiler summary/provenance code as needed. Ask the primary agent if module
constant identity cannot be represented without a new language-level rule.
