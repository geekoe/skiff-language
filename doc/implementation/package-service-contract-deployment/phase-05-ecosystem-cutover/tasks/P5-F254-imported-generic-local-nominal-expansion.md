# P5-F254 Imported generic expansion with local nominal arguments

## Context

F251 publishes a unified WebSocket ingress operation:

```skiff
function websocket(
  event: std.websocket.WebSocketIngressEvent<AihubSocketContext>
) -> std.websocket.WebSocketConnectResult<AihubSocketContext>?
```

The source model treats the instantiated input as opaque and reports unknown
fields `tag`, `connectRequest` and `receiveEvent`. The std FileIR descriptor is
a discriminator union:

```text
{ tag: "connect", connectRequest: WebSocketConnectRequest }
| { tag: "receive", receiveEvent: WebSocketReceiveEvent<Context> }
```

The official `<null>` fixture works; instantiation with a service-local nominal
context fails. Agine has the same shape with its local `ConnectionContext`.

## Required implementation

- Instantiate imported Package generic descriptors when type arguments include
  consumer-local nominal types.
- Substitute the local nominal consistently through union branches, fields,
  nullable/container nesting and return types.
- Preserve the imported generic owner/key/type identity and the local
  argument's nominal identity.
- Make discriminator narrowing expose branch fields.
- Reject wrong arity, unresolved local arguments and another nominal argument
  where the exact instantiation differs.
- Do not special-case WebSocket names or structurally erase the local nominal.

## Acceptance

- Focused imported-generic fixtures cover null, builtin, local record nominal,
  nested containers and discriminated union narrowing.
- AIHub and Agine unified WebSocket operations compile with their local context
  types.
- Relevant source/lowering/schema tests, workspace check and diff check pass.
- F251 operations become source-valid; continue Available/deployment receipt.
- Result and commit; no push, stable operation or disk cleanup.
