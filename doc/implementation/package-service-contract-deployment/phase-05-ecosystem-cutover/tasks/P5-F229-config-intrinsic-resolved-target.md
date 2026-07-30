# P5-F229 Config intrinsic resolved target

## Context

Relay v1Proxy contains direct `config.optional<string>(...)`. Callable-effects
recognizes the direct intrinsic and returns Fresh/no effects, but
resolved-call target collection records the same call as
`Unknown(UnresolvedName)`. Compiled semantic facts publish that Unknown and
boundary eligibility rejects the operation independently of aggregate effects.

Do not hide Unknown targets in eligibility.

## Required implementation

1. Add explicit `ResolvedCallTarget::ConfigIntrinsic` carrying exact
   require/optional/has kind.
2. Resolve only direct canonical `config.require<T>`, `config.optional<T>`, and
   `config.has` syntax before dependency/local/native lookup.
3. Aliases, indirect calls, shadowed local `config`, and unknown config methods
   must not be misclassified.
4. Callable-effects consumes the typed target as Fresh/no effects; remove or
   assert consistency with the duplicate AST fast path.
5. Config intrinsic creates no callable graph edge/source callable key and is
   not published as an external/unknown CallableTargetFact.
6. Preserve config requirement collection, validation, lowering, and Runtime
   behavior.

## Acceptance

- require/optional/has call keys resolve as ConfigIntrinsic with Fresh/no
  effects and no Unknown fact.
- Negative shadowing/alias/indirect/unknown-method tests remain fail-closed.
- A public config.optional caller is boundary Available while retaining its
  config requirement.
- Fresh Relay removes preorder 204 Unknown and preserves exactly 17 paths;
  remaining independent effects are reported separately.
- Source/compiled/projection tests, workspace check, diff check, result, commit.
- No push or stable operations.
