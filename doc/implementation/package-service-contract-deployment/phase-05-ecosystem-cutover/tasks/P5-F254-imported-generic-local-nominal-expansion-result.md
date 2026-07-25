# P5-F254 Imported generic expansion with local nominal arguments result

## Result

Completed. Imported generic types now recover their concrete arguments before
expanding their source-visible shape. Package/prelude descriptors are first
resolved with their declared parameters as `TypeParam` IR and are then
instantiated by IR substitution.

This ordering preserves both sides of the type:

- the imported generic keeps its Package/prelude owner and public type name;
- a consumer-local argument remains its exact `LocalType` nominal identity.

The substitution recursively crosses unions, records, nullable values and
containers. It therefore exposes discriminator fields and the selected branch
fields without structurally replacing the local nominal argument.

Package and prelude types now also reject missing or excess type arguments at
source resolution. An unresolved argument still fails at its own resolution
site, and assignability continues to distinguish two different local nominal
arguments.

## Coverage

`compiler/tests/websocket_ingress.rs` now covers:

- the existing `<null>` WebSocket fixture;
- a builtin `string` argument;
- a local record nominal argument;
- `Array<Context?>` through nested imported generics;
- `IngressEvent -> ReceiveEvent -> Connection -> context` substitution;
- discriminator narrowing and `connectRequest` / `receiveEvent` field access;
- a local nominal argument retained in the lowered generic execution type;
- zero and two argument arity rejection;
- unresolved argument rejection;
- rejection when an instantiated field supplies a different nominal type;
- exact Available projection and deployment for the focused nominal operation.

Validation:

- `cargo test -p skiff-compiler-source`: passed
- `cargo test -p skiff-compiler --test websocket_ingress`: 4 passed
- `cargo test -p skiff-compiler --test package_std_schema`: 8 passed
- `cargo check --workspace`: passed
- `git diff --check`: passed

The full lowering crate run reached 41 passed and one failure that reproduces
unchanged on integration HEAD `df9f228`: its
`exact_interface_and_impl_contract_types_share_opaque_execution_projection`
test still expects a nested exact PackageSymbol to be erased to `unknown`.
That baseline failure is independent of this source-shape change; the focused
WebSocket integration test exercises source, schema, lowering, projection and
deployment together and passes.

## Ecosystem receipts

AIHub F251 source crossed the unified WebSocket operation and no longer
reported unknown `tag`, `connectRequest` or `receiveEvent` fields. Its package
build then stopped at the independent F253 site:
`handleAihubHttp is boundary unavailable`.

Agine's checked-in F251 tree was initially stopped before compilation by the
obsolete `packages` section in `config.dev.yml`. A temporary source-identical
copy without that obsolete profile file then stopped while loading an older
`agine.ai/agent` artifact whose `AnyInterface` descriptor is not usable by the
current compiler. Neither failure is a WebSocket source diagnostic. F251 owns
the fresh unified AIHub/Agine build and Available/deployment receipts after
this compiler commit is integrated.
