# Cross-System Fixtures

This directory owns golden cases for behavior shared by multiple Skiff
subsystems. Fixtures here should describe protocol-level expectations rather
than one component's private implementation details.

Use this directory when the same case must be consumed by at least two of the
compiler, runtime, router, registry, or tooling test suites.

Each fixture case must include an `appliesTo` array. The array names the systems
that must consume the case, and each consumer should filter out cases that do
not name that system. Allowed system names are `compiler`, `runtime`, and
`router`. Each case must name at least two distinct systems so it remains a
cross-system expectation rather than a private unit test fixture.

Publication id fixtures describe URL-like semantic ids plus their storage-safe
projection. They intentionally do not include stable-token hashes. Skiff stores
the URL-like id in config and protocol fields, then projects it only for artifact
path segments, runtime target components, and service database names.
