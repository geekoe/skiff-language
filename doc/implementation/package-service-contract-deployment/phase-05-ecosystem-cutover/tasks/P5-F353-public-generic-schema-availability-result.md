# P5-F353 Public Generic Schema Availability Result

Status: Completed

## Exact checkpoints

- Task base: `acbb4d7ea1174289c9c89c93b866dd1511815e17`
  (`e21f0cca314e408890631e1f8c09f6b34a4ed5b9`).
- Initial P5-F353 production/tests:
  `99570bb290cddeddf07594bfdaf2da57d68f39d3`
  (`e79c866e4f9a37cdff26080f5dc6ab35fe79e718`).
- Initial F352/F354 integration checkpoint:
  `f129bc7a8d18fef8d7ec6fca587e6332fd73cd3d`
  (`a9b0d2f15d42bb87f15ec14023cad52fdf171a48`).
- P5-F353 review follow-up:
  `b30bc37e5d4d6252fd0b07097a281f78402fa186`
  (`e9556da1a5279386c3cc5eca2b902870d502e607`).
- Final F352 production checkpoint:
  `f2e5b6daa08a5ec261ca2374dc737d3d8996cb3f`
  (`fd032f9714243f687fbed0d903cae28b17e67ab9`), integrated by
  `1d9cd7341270d065f320c7b2d5efb433fc0e2e7e`
  (`5e9e906b56fb970d8d7b784bd29c420d8edae055`).
- Final local integration checkpoint:
  `afe6871bf65a4182201bfa9527d10ed69d019b35`
  (`532065bf0a7aff80670684c2a357d3897f0cf0b2`).
- Final checkpoint fixture adaptation:
  `34861ade195c07b4fbef566b8d76452862c7ca7c`
  (`8dda0e1853ac2733ce98f66714ff4ee89e47de14`).
- Branch/worktree: `codex/p5-f353-generic-schema` at
  `/Users/geek/workspace/skiff-p5-f353-generic-schema`.

## Outcome

- Public generic records, representations, named unions, and interfaces remain
  exact `PackageLocalAbi` symbols with implementation links.
- Package schema eligibility is a whole-owner decision. A declaration with
  type parameters, a free `TypeParam`, an `AppliedNominal`, or a transitive
  reference to such a declaration emits no schema index entry, record, or
  source reference. Interface method signatures and `implicit_self` participate
  in the same closure.
- Public callable signatures retain their complete local ABI shapes.
  Generic parameter, return, stream, and callback shapes project as structured
  `Unavailable(UnsupportedBoundaryType)`. A callback's transitive generic
  closure failure takes precedence over the callback-adapter fallback.
- The former name-based `std.websocket` service-boundary admission is absent.
  Source/prelude generic parsing and execution type arguments remain intact.
- Closed schema types remain exact. The compiler fixture now closes through
  `std.service.InternalError`, while generic `std.websocket` declarations remain
  local-ABI/linkable and schema-ineligible.
- Existing strict package-schema and artifact-identity admission remains
  fail-closed for forged generics, incomplete transitive record closures,
  invalid applied nominal arity/scope, and identity tampering.

## Validation evidence

| Command | Result |
| --- | --- |
| `cargo test -p skiff-compiler-projection package_schema -- --list` | PASS, 13 tests listed |
| `cargo test -p skiff-compiler public_generic -- --list` | PASS, 2 tests listed |
| `cargo test -p skiff-compiler-projection package_schema` | PASS, 13/13 |
| `cargo test -p skiff-compiler public_generic` | PASS, 2/2 |
| `cargo test -p skiff-compiler package_imports` | PASS, 1/1 selected |
| `cargo fmt -p skiff-compiler-projection -p skiff-compiler -- --check` | PASS |
| `git diff --check` | PASS |
| `cargo test -p skiff-compiler-projection` | PASS, 57/57 |
| `cargo test -p skiff-compiler --test websocket_ingress` | PASS, 4/4 |
| `cargo test -p skiff-compiler-projection-input rejects_forged_generic_package_schema_before_exposing_index_or_records` | PASS, 1/1 |
| `cargo test -p skiff-compiler-projection-input accepts_exact_cross_package_transitive_closure_and_rejects_missing_or_extra_records` | PASS, 1/1 |
| `cargo test -p skiff-artifact-identity applied_nominal_argument` | PASS, 2/2 |
| `cargo test -p skiff-artifact-identity package_artifact_admission_rejects_empty_and_applied_package_schema` | PASS, 1/1 |
| `cargo test -p skiff-compiler-source invalid_applied_nominal_bases_arity_and_type_param_scope_fail_closed` | PASS, 1/1 |

The required selectors were non-empty. Compiler/source and runtime-linker emitted
their existing warnings only; no validation failure remained.

## Reverse scan and scope

- No `PackageBoundaryKind::PackageSchema` or
  `contains_generic_boundary_shape` residual remains.
- The remaining `std.websocket` production match is an explicit unsupported
  boundary classification, not an allowlist.
- Schema records, index entries, and source refs are derived from the same
  complete eligible-owner set; no partial schema path remains.
- P5-F353 production/tests changes stayed in compiler projection and compiler
  tests/fixtures. No gateway, router, runtime, stable instance, live service,
  workspace-wide test, root test, or push was used.

## Out-of-scope observation

- A bounded WebSocket-fixture probe found that, after the final F352 checkpoint,
  a package with a contract dependency cannot type-resolve calls to
  `std.time.sleep` or `std.websocket.sendTextToConnection`: source compilation
  reports `dependency alias std has no resolved package owner`.
- The failing path reaches
  `compiler/source/src/type_resolution_model/shape_assignability.rs`; source
  compilation supplies declared `dependency_packages` to type resolution while
  compiler-owned std is selected through `available_packages`.
- This is outside P5-F353's authorized projection owner and is not required by
  its completion matrix. The WebSocket fixture was kept focused on generic
  source parsing, arity, execution shape, and structured boundary
  unavailability. Resolving std callable ownership in contract-dependent source
  compilation, if required, should be a separate F352/source-owner DAG node.
