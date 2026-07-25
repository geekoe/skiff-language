# P5-F206 Cross-package test-double sequence

## Parent

P5-H31-r05 batch handoff, Phase 05 ecosystem cutover.

## Context

Account's canonical package test has one remaining failure. Its
`dnsTxtVerified` path intentionally performs three HTTP resolver requests, and
the committed test fixture supplies a three-entry negative response sequence.

When a test overlay calls a production-package function,
`runtime/eval/src/service_dispatch.rs::outbound_test_effect_doubles` calls
`next_test_effect_double` only once per target and constructs a one-entry
vector. The remaining two entries stay in the overlay registry. The production
package consumes the first double, then its second and third requests have no
forwarded double and attempt real network access or time out.

Test mode must forward the exact committed sequence across the package boundary
without allowing real external effects.

## Required implementation

1. Transfer the complete remaining committed test-double sequence for the exact
   outbound target when dispatching from a test overlay to production code.
2. Preserve ordering and one-shot consumption for every sequence item.
3. Transfer only the exact target's entries; do not merge doubles for other
   native targets or package calls.
4. Consumption must remain isolated to the current test execution and must not
   leak between cases, retries, packages, or sessions.
5. In test mode, missing or exhausted committed doubles must fail closed with a
   precise test-effect error. They must never fall through to real network,
   filesystem, clock, or other external effects.
6. Preserve non-test production dispatch behavior.

## Acceptance

- Runtime tests cover a three-entry cross-package sequence in exact order.
- Negative tests cover exhaustion, wrong target, duplicate/retry isolation, and
  prove no real native handler is invoked.
- Existing single-entry and direct-overlay test-double behavior remains green.
- Account's committed three-entry fixture is retained.
- Real Account package tests pass all 19 cases.
- Relevant Runtime tests, `cargo check --workspace`, and `git diff --check`
  pass.
- Add `P5-F206-cross-package-test-double-sequence-result.md`.
- Commit the work; do not push and do not operate the shared stable instance.

## Authority

Use this task as immediate authority. Follow the Runtime test-effect registry
and service dispatch paths it directly invokes. Ask the primary agent if exact
sequence ownership cannot be preserved without changing the test artifact
format.
