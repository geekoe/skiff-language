# P5-F359 HTTP gateway request protocol result

状态：Completed（C3 shared Rust/TypeScript wire checkpoint；Host、Router builder/dispatch与
test-runner consumer仍未迁移，因此不表示external HTTP request已经可执行）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `d26b56e78871c86b783944c54971886defa71e98` | `40b18203fefa395cffd9dc47fe8b088a8147cecd` |
| task checkpoint | `d1afce0f118279c278ef096bcc3ebab7943c70b9` | `619f68e6f9d785d9859ade5aa74dbb1da2b268f6` |
| production/tests | `7c2161c788604165480563b6dba30dd4a2b66272` | `e76455d75d2d7b18b0928d713e7b538739ee540b` |

工作分支为`codex/p5-f359-gateway-request-protocol`，worktree为
`/Users/geek/workspace/skiff-p5-f359-gateway-request-protocol`。本leaf没有merge/rebase
integration，没有运行workspace/root或stable/live，没有修改lockfile、Host、Router
registry/dispatch/gateway、test-runner或三仓库service源码，也没有push。

## 2. Final canonical field sets

Canonical binary frame继续使用`skiff-runtime-frame-v1`、`request.start`和既有binary framing。
normalized typed header的精确shape为：

| owner | required fields | optional fields |
| --- | --- | --- |
| header | `schemaVersion`, `type`, `requestId`, `mode`, `caller`, `routing`, `trace`, `httpRequest` | `clientSession`, `deadline`, `testEffectsEnabled` |
| caller | `kind` | none |
| routing | `kind`, `assemblyIdentity`, `assemblyGeneration`, `gatewayEntryIdentity`, `ingress` | none |
| ingress | `protocol`, `host`, `method`, `path` | none |
| clientSession | `id` | none |
| deadline | `timeoutMs`, `expiresAt` | none |
| trace | `traceId`, `spanId` | `parentSpanId`, `sampled` |
| httpRequest | `method`, `url`, `path`, `query`, `headers` | none |
| query/header item | `name`, `value` | none |

- `caller`精确为`{ kind: "gateway" }`；`caller.target`不能进入canonical decoder。
- `routing.kind`精确为`runtimeAssembly`。`assemblyIdentity`是artifact-model
  `AssemblyIdentity`且只接受v2；`gatewayEntryIdentity`是artifact-model
  `GatewayEntryIdentity`且只接受`skiff-gateway-entry-v1:sha256`。
- `assemblyGeneration`和`deadline.timeoutMs`必须是非负safe integer并拒绝`-0`。
- ingress只允许`protocol: "http"`；`host`和`method`非空，`path`必须以`/`开头。
- `mode`只允许`unary | serverStream`。本leaf不选择stream codec、framing或deadline执行责任。
- `testEffectsEnabled`是wire optional、typed default false；Rust encode省略false，Rust和TypeScript
  decode都把缺失值归一化为false。
- binary payload始终是opaque HTTP request body bytes；空、非空、非UTF-8和serverStream body均
  不由transport/protocol owner解释。
- `contractOperationId`、top-level `gatewayEntryIdentity`、activation/business/WebSocket字段、
  handler/pre/guard/adapter facts和`testEffectDoubles`都不属于canonical field set。

## 3. Rust唯一strict decoder

- `runtime/transport/src/runtime_assembly_request.rs`只声明上述HTTP header、caller、routing和
  ingress typed surface；旧operation routing、RuntimeAssembly WebSocket、adapter、activation和
  test-double decode路径已删除。
- routing直接使用artifact-model `AssemblyIdentity`与`GatewayEntryIdentity`。lexical模块复用
  artifact-model v2 assembly validation、activation generation validation和
  `GatewayEntryIdentity::parse`，没有alias、dual read或fallback。
- metadata模块只保留client session、deadline、trace、HTTP request及name/value item职责；
  nested object均`deny_unknown_fields`，present optional不能用`null`代替缺失。
- existing strict JSON owner继续先拒绝duplicate/escaped duplicate keys、invalid UTF-8/JSON和
  trailing JSON，再进入typed serde decode；binary frame magic/version/encoding/length仍fail
  closed。
- canonical typed encode/decode精确roundtrip header与opaque payload。独立legacy
  `RequestStartFrameHeader` baseline仍由旧decoder接受，同时被canonical decoder拒绝。

## 4. TypeScript唯一validator/decoder

- `runtimeAssemblyRequest.ts`拥有唯一canonical field/routing/ingress validator；
  `runtimeAssemblyRequestMetadata.ts`只拥有request/lifecycle metadata；
  `runtimeAssemblyRequestStrict.ts`、`runtimeAssemblyRequestJson.ts`和
  `runtimeAssemblyRequestFrame.ts`继续拥有strict raw JSON与binary frame decode。
- TypeScript与Rust逐字段同构并执行相同unknown/missing/type/nullability、safe integer、
  identity、HTTP-only与opaque payload约束。Router-to-runtime接受canonical header，
  runtime-to-router明确拒绝。
- `runtimeProtocol.ts`只做canonical schema注册、thin delegation/default normalization与独立
  general legacy路径。declarative `request.start` schema现在是互斥的legacy/canonical HTTP
  `oneOf`；shared canonical与legacy corpus都只命中各自分支。
- `envelope.ts`的Router-to-runtime union包含normalized canonical typed header；
  runtime-to-router union没有加入该header。旧legacy DTO继续独立存在，没有被canonical decoder
  当作正例。
- protocol owner没有读取snapshot、deployment、ServiceContract或handler selection。

## 5. RuntimeAssembly identity v2 consumer convergence

- `assemblyActivationLexical.ts`是TypeScript唯一RuntimeAssembly identity lexical owner，合法
  prefix与错误文本均升级为`skiff-runtime-assembly-v2:sha256`。
- request validator和WebSocket generation lifecycle tuple直接复用该owner；lifecycle
  generation也改为复用`activationGeneration`，不再复制identity/generation regex。
- activation request/state/control/raw cases、runtime activation binary frame、ecosystem store、
  WebSocket lifecycle wire，以及直接artifact-model/runtime-transport/Router protocol tests中的
  合法当前identity均机械迁到v2。
- canonical empty assembly oracle为
  `skiff-runtime-assembly-v2:sha256:247fc2b3714bf715dc7918a10618be49493645efbbc0f293fc7b3d2e4d32b50f`；
  ecosystem store corpus中的8个引用全部使用该真实F358结果。
- scoped v1反搜仅剩3个明确拒绝用例：activation raw v1、request typed mutation v1和request raw
  v1。没有v1合法正例。

## 6. Shared cross-system corpus

`runtime-request-wire.json`和checkpoint已重写为HTTP-only contract，Rust tests与TypeScript
verifier消费同一集合：

| 集合 | 实际数量 | 覆盖 |
| --- | ---: | --- |
| canonical headers | 3 | full metadata/nonempty body基准、minimal/defaults、`unary`与`serverStream` |
| typed mutations | 114 | required/optional/unknown/type/nullability、identity、selector与旧static facts |
| raw/frame cases | 19 | duplicate/escaped duplicate、`-0`、unsafe integer、v1、UTF-8/JSON和binary framing |
| payload cases | 4 | empty、nonempty、non-UTF-8、serverStream opaque body |
| default equivalence pairs | 1 | absent `testEffectsEnabled`等价于false |
| legacy ordinary headers | 1 | legacy-only accepted、canonical rejected |

138个具名mutation/raw/payload/equivalence case全局唯一；两端均检查所有集合非空、所有
`baseIndex`有效且每个case真实执行。每个canonical header还逐一证明Router-to-runtime接受、
runtime-to-router拒绝。verifier不冻结方便的case数量，而是在结果中输出实际数量。

## 7. Reverse-search classification

Canonical Rust/TypeScript standalone modules上的下列搜索为零匹配：

```text
ContractOperationId|contractOperationId|activationIdentity|testEffectDoubles|
websocketAdapter|webSocket|businessIdentity|httpAdapter|handler|guard|adapterArgs
```

`runtime-request-wire.json`中的旧名命中全部是明确reject mutation：

- top-level `activationIdentity`、`gatewayEntryIdentity`、`businessIdentity`、
  `websocketEntryId`、`httpAdapter`、`websocketAdapter`和`testEffectDoubles`；
- `caller.target`（值为`forged-handler`）；
- routing `contractOperationId`、`gatewayEntryKey`、`deployment`和ServiceContract字段；
- ingress `webSocket`。

该corpus中的其余`gatewayEntryIdentity`均位于required `routing`内。`runtimeProtocol.ts`中的旧
operation/adapter字段只属于保留的general legacy schema branch，不属于canonical validator；
declarative schema test证明两分支互斥。

## 8. Selector与验证

Selector先枚举并确认非零：

| selector | 枚举结果 |
| --- | ---: |
| `skiff-runtime-transport runtime_assembly_request` | 7 tests |
| `skiff-artifact-model assembly_activation` | 5 tests |
| `skiff-runtime-transport assembly_activation` | 2 tests |
| `skiff-runtime-transport websocket_generation_lifecycle` | 5 tests |

| 命令 | 结果 |
| --- | --- |
| 四条`cargo test ... -- --list` | PASS；7 / 5 / 2 / 5，均非零 |
| `cargo test -p skiff-runtime-transport runtime_assembly_request` | PASS；7 selected |
| `cargo test -p skiff-artifact-model assembly_activation` | PASS；5 selected |
| `cargo test -p skiff-runtime-transport assembly_activation` | PASS；2 selected |
| `cargo test -p skiff-runtime-transport websocket_generation_lifecycle` | PASS；5 selected |
| `pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts` | PASS；48 tests |
| `node cross-system-fixtures/package-service-ecosystem/verify.mjs --runtime-wire-self-test` | PASS；输出3 / 114 / 19 / 4 / 1 / 1 |
| `cargo check -p skiff-runtime-transport` | PASS |
| changed Rust `rustfmt --edition 2021 --check` | PASS |
| `git diff --check` | PASS |

补充identity consumer证据也通过：full ecosystem verifier、两个直接Router
activation/lifecycle test files（16 tests）以及transport actor activation identity corpus
test（1 test）。

## 9. Downstream compile breaks

完整`pnpm --filter @skiff/router type-check`按任务预期exit 2；canonical protocol owner路径没有
报错，49个错误全部位于尚未迁移且本leaf禁止修改的consumer/tests：

| 精确consumer | 错误数 | 仍消费的旧shape |
| --- | ---: | --- |
| `src/router/assemblyRuntimeRegistry.ts:529-633` | 10 | `testEffectDoubles`、`caller.target`、nullable method、`contractOperationId`、adapter/WebSocket/top-level identity |
| `src/router/runtimeDispatcher.ts:333-1141` | 18 | WebSocket ingress/adapter、`contractOperationId`、WebSocket/top-level identity |
| `tests/assembly-websocket-gateway.test.ts:128-523` | 17 | WebSocket adapter/entry旧request header |
| `tests/loop-risk-health.test.ts:697` | 1 | legacy-only `RequestStartFrameHeader` assignment |
| `tests/router-websocket-trust-dispatch.test.ts:325-331` | 3 | `caller.target`、WebSocket ingress与nullable method |

这些断点是后续Router snapshot/builder/HTTP/WebSocket dispatch consumer owner的迁移证据；本leaf
没有越界修改它们来制造workspace绿色。

## 10. 自验收矩阵

| 任务条款 | 代码/语料证据 | 验证证据 |
| --- | --- | --- |
| canonical routing/header | Rust typed structs、TS interfaces/validators、checkpoint exact field declarations | 3 canonical headers、114 mutations、两端roundtrip |
| strict Rust decoder | typed artifact identities、serde exact objects、strict raw JSON/frame owner | 7 Rust request tests，含duplicate、`-0`、unsafe integer、legacy互斥 |
| strict TS decoder | split canonical/metadata/strict/json/frame owners；runtimeProtocol thin delegation | 48 protocol tests与shared verifier |
| HTTP-only/opaque payload | ingress enum/validator只接受HTTP；frame decoder不读payload layout | WebSocket mutation rejected；4 payload cases roundtrip |
| direction | Router-to-runtime typed union；runtime-to-router未注册 | 每个canonical正例逐一验证正向accept、反向reject |
| identity v2 | unique TS lexical owner及直接fixture/wire consumers | activation、transport、lifecycle聚焦tests与scoped v1反搜 |
| unfrozen corpus execution | nonzero/unique/base-index assertions，无固定mutation数量 | Rust和TS实际执行全部138个具名case |
| boundary containment | 未改Host、Router consumer/dispatch、test-runner、response/cancel/connection wire | downstream type-check错误精确归档；owned protocol paths零错误 |

本checkpoint没有决定typed JSON stream支持、stream response framing、deadline执行责任、HTTP
runtime codec或WebSocket application message routing。
