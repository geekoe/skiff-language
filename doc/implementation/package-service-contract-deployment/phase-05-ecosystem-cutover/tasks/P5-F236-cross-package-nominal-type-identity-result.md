# P5-F236 Cross-package nominal type identity result

## Root cause and fix

The first real AIHub mismatch was not two structurally different
`HttpHeader`s. It was one canonical Package schema nominal and one unresolved
local `PackageSymbol` that the diagnostic rendered with the same source text.

`contract_type_resolution::resolve_expanded_expr` canonicalized direct
dependency symbols but omitted the `PackageRefIr::PackageId` branch used by
compiler-owned std. The same pipeline also resolved only contract-required
owners before source compilation, although exact direct manifest dependencies
are required there too.

The shared fix now:

- resolves every exact direct dependency schema before source compilation;
- canonicalizes both dependency aliases and exact package-ID symbols through
  verified schema records;
- indexes direct Package records by their public manifest path while retaining
  the stable schema key and type ID;
- preserves a package nominal while projecting fields from a canonical record;
- emits the complete `PackageTypeRef` trees when ordinary display names hide a
  canonical mismatch.

No display-string comparison, identity erasure, latest selection or AIHub
conversion was added.

## First real mismatch identities

The first sites were
`aihub/service/internal/aihub_service.skiff:89`, `:93-95`, `:110-120`,
`:460-482` and the caught-response returns at `:2292-2332`.

Contract/expected side:

```text
origin: Relay ServiceContract schema
owner: skiff.run/std
version: 1.0.0
build: skiff-package-build-v4:sha256:ad74528a916c42ed45df872d4bbb591c778f571de803a164c01cf96697ec206b
local ABI: skiff-package-local-abi-v3:sha256:416c423e0a87029e6cd5c6b484f8cb960a31eb75929bd4bb39bdbcd212c8e855
HttpHeader type ID: skiff-package-schema-type-v1:sha256:449c7e8a0201bf098ba3487b053f524896c36d619779ed6441bad16e922ff5a9
HttpRequest type ID: skiff-package-schema-type-v1:sha256:8cb6e97b3bfdbfe6591cf4ad011d854fae9d557d1db7b2b759bfdafeceeeb4c3
HttpResponse type ID: skiff-package-schema-type-v1:sha256:4d9e3c16a83f23eb333a19d9641b6971363943aebe3f2d45643a1f457a1644c8
generic: Array<PackageSchema(skiff.run/std, std.http.HttpHeader, 449c...)>
```

Source/actual side before the fix:

```text
origin: compiler-owned std source PackageId symbol
canonical candidate version/build/ABI:
  skiff.run/std@1.0.0
  skiff-package-build-v4:sha256:d1a08e08d04613b5950fee4b3b7bd0c3118226527032ce1deb794dbba23c11b8
  skiff-package-local-abi-v3:sha256:ecd876988394d2b6ea7f6e46c4160f5b22c44d281fc4c0f16aa50d5e33956267
schema type IDs: the same exact 449c..., 8cb6... and 4d9e...
internal generic before fix:
  Array<Local(PackageSymbol(PackageId(skiff.run/std), std.http.HttpHeader))>
```

Thus the artifact build/ABI differed in the retained cross-task store, but the
schema owner/key/type IDs were identical. The compiler failure itself was the
loss of the canonical nominal on the source side; exact binding validation
continues to reject a mismatched version/build/ABI before these types are
compared.

## llmApi audit and real gate

After the std repair all nominally identical std errors disappear. The llmApi
errors share the same lost-nominal family but expose two additional paths:

- direct dependency schemas were not pre-resolved or indexed by public path;
- field projection expanded package aliases such as `LlmApiFormat` into local
  literal unions.

Those paths now retain the exact llmApi owner/key/type IDs. Real AIHub advances
to the next independent target-typing/representation gate: literal values and
expanded alias unions are not yet admitted as values of package nominal aliases
(`LlmRole`, `LlmReasoningLevel`, `LlmApiFormat`), plus existing untyped object
literal and iterable diagnostics. This is no longer a same-name canonical
identity mismatch.

The exact retained graph was used read-only. Relay remained build
`6cc37cd...`; no stable instance, push, source workaround or disk cleanup was
performed.

## Validation

- focused exact owner/key/type-ID assignability tests: 3 passed;
- complete compiler-source library: 273 passed;
- compiler pipeline tests: passed;
- `cargo check --workspace`: passed;
- `git diff --check`: passed.

Existing pre-source binding tests cover version/build/local-ABI mismatch and
duplicate exact bindings; owner and schema type-ID negatives remain covered by
the focused nominal assignability matrix. The full compiler library again has
the integration baseline std golden mismatch (`4cf082...` expected,
`d1a08e...` actual); the other 17 tests passed.
