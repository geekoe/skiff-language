# P5-F273 Public alias expansion

## Authority

`doc/reference/static-semantics.md`

## Problem

Skiff `alias` declarations are transparent type abbreviations. Public package
projection currently emits some aliases as nominal `PackageSchema` types.
Consumers therefore cannot use an imported alias according to its RHS type.

Confirmed case:

`agent canonical.SubagentExecutionStatus` aliases a string literal union but is
projected as a nominal schema, so Agine cannot assign it to or pass it as
`string?`.

## Required implementation

- Expand aliases to their exact RHS before executable IR, public callable
  signatures, operation contracts, usage descriptors and schema closure are
  finalized.
- Preserve literal unions, containers, callbacks and nested external package
  references recursively.
- Do not emit a nominal schema identity solely for an alias.
- Keep record, error, actor and other genuinely nominal declarations nominal.
- Reject alias cycles and missing or ambiguous RHS references fail closed.

## Acceptance

- Local, exported and imported alias matrices cover scalar, literal union,
  nested container and cross-Package RHS types.
- Nominal record/error projection remains unchanged.
- Fresh Agent publication and Agine type-check cross the
  `SubagentExecutionStatus` sites.
- Projection/source suites, workspace check, result and commit.
