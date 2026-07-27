# P5-F426A WebSocket connect current wire checkpoint result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。

本 leaf 已冻结 TypeScript/Rust 共用的 current `websocketConnect` request/response wire，并保留
HTTP 已有 canonical bytes。没有实现 Runtime/Host connect execution、Router gateway/dispatcher、
handler synthesized accept、artifact/compiler/deployment authoring、stable/live 或后继 consumer。

## 1. 精确输入与提交

| 项 | commit | tree |
| --- | --- | --- |
| 父节点指定输入 | `5e74633f44e9770e92b6a5866682d5eab65e2053` | `b0fbf41a17ef09bdeee91973df0aa8e625aaa0c4` |
| implementation | `68062a4dd7fd0264281bc3349e94fda005f9aad3` | `4c217a87b4c79c1104577db172a4a74ce7f73afe` |

implementation 提交后 worktree clean；本 result 单独提交。

## 2. 冻结的 current wire

### 2.1 Request closed union

- `request.start` 的 current runtimeAssembly wire 是按
  `routing.ingress.protocol` 判别的 closed union：
  - HTTP：`protocol: "http"`、非空 string `method`、exact `httpRequest`；
  - connect：`protocol: "webSocket"`、`method: null`、exact `websocketConnect`。
- connect 只允许 `unary`，binary payload 必须为空。
- connect metadata 精确包含 `connectionId`、`url`、`query`、`headers`、`cookies`、可选
  `version`、canonical `websocketEntryId` 和 `gatewayEntryIdentity`。
- metadata 与 routing 的 `gatewayEntryIdentity` 必须精确相等。
- `connectionId` 使用已有 generation wire 的 canonical ASCII spelling，长度为 1–255。
- HTTP producer/consumer 的命名类型保持不变；通用 current reader 返回 exact HTTP/connect union。
  已有 HTTP-only validator 对 connect 显式 fail closed，没有偷跑执行。

### 2.2 Response closed union

- connect response 只能是 `response.end`、`payloadPresent: false`、空 binary payload。
- `accept` 只有可选 `businessIdentity` 和可选 `connectionPolicy`。
- `reject` 只有必需的 unsigned-u16 `code` 与 string `reason`。
- policy 只有 non-zero-u32 `maxConnections`、`close-oldest | reject-new`、可选 u16
  `closeCode` 和可选 string `closeReason`。
- exact TS/Rust response readers要求 `websocketConnect` 存在，不把普通 `response.end` 推断成
  accept，也没有 handler synthesized accept。

Request/response current production DTO 与 exact readers 均不含 body business schema、
receive/message、Context、context codec、payload context、operation ABI 或
`ContractOperationId`。

## 3. Language-neutral exact JSON parity

唯一权威 corpus 是
`cross-system-fixtures/package-service-ecosystem/runtime-websocket-connect-wire.json`。TS 与 Rust 都读取
同一份 `canonicalJson`，并比较完整 JSON string；没有各自生成 hash 或默认事实。

### 3.1 Request vectors

| vector | canonical JSON bytes | payload | 结果 |
| --- | ---: | ---: | --- |
| `canonical HTTP bytes` | 710 | 0 | TS/Rust exact |
| `websocketConnect full metadata` | 1348 | 0 | TS/Rust exact |
| `websocketConnect optional metadata absent` | 996 | 0 | TS/Rust exact |

HTTP vector与既有 `runtime-request-wire.json` minimal HTTP header经既有
`testEffectsEnabled: false` normalization后的 710 bytes逐字节相同。

full connect vector复用 F425A 冻结的 canonical identities：

```text
skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936
skiff-websocket-entry-v1:sha256:3a0f9b39b684e0c324ff3f729395273987f86ed648e6c0ddd0cb35b67b1aa616
```

### 3.2 Response vectors

| vector | canonical JSON bytes | payload | 结果 |
| --- | ---: | ---: | --- |
| `websocketConnect accept minimal` | 174 | 0 | TS/Rust exact |
| `websocketConnect accept full` | 320 | 0 | TS/Rust exact |
| `websocketConnect reject` | 196 | 0 | TS/Rust exact |

共享 corpus 另含 24 个 request mutation 和 25 个 response mutation，覆盖 unknown field、wrong
discriminator、missing/mismatched/noncanonical identity、HTTP/WS metadata混搭、非空 payload、
legacy body/receive/message/context/operation shape，以及 accept/reject field交叉污染。

## 4. 自验收矩阵

| 完成标准 | 状态 | 证据 |
| --- | --- | --- |
| HTTP/connect closed discriminated union | PASS | TS `RuntimeAssemblyRequestStartFrameWireHeader`；Rust同名 enum；shared 3-vector corpus |
| HTTP canonical bytes不变 | PASS | 既有 minimal HTTP normalization与新 golden均为 exact 710 bytes |
| connect exact routing与metadata | PASS | branch-specific strict DTO、canonical identity reader、identity equality assertion |
| request无body/receive/message/Context/operation | PASS | exact allowlist + shared negative mutations + production reverse search |
| response exact accept/reject且无context/message | PASS | TS/Rust tagged union、空 payload reader、25 response mutations |
| strict reject unknown/wrong/missing/mix/noncanonical | PASS | TS 56-case suite；Rust shared mutation loops与duplicate-key probes |
| TS/Rust exact canonical JSON parity | PASS | 6 language-neutral `canonicalJson` vectors逐 string比较 |
| 不产生 synthesized accept | PASS | wire reader只解析显式 `websocketConnect.result`；未修改 execution owner |
| 未越过 execution/authoring scope | PASS | 15 个 implementation files全部位于 leaf允许范围 |

## 5. 验证证据

### 5.1 Green focused verification

| 命令 / suite | 结果 |
| --- | --- |
| `cargo test -p skiff-runtime-transport runtime_assembly_request` | PASS：13；另有 integration binary 0 matched、2 filtered |
| `cargo test -p skiff-runtime-transport` | PASS：88 unit + 2 integration |
| `pnpm --dir router test runtime-assembly-request` | PASS：1 file，56 tests |
| `pnpm --dir router exec vitest run tests/protocol.test.ts tests/runtime-protocol-websocket-response.test.ts --reporter=dot` | PASS：2 files，103 tests |
| `pnpm --dir router exec tsc --noEmit` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

### 5.2 任务字面命令的 discovery classification

任务列出的 `pnpm --dir router test -- runtime-assembly-request` 已在 implementation tree原样执行。
当前 pnpm/Vitest组合把额外的 `--` 传成：

```text
vitest run --exclude 'dist/**' -- runtime-assembly-request
```

该 spelling 没有应用 filename filter，而是发现整个 Router inventory：51 files、687 tests。结果为
49 files / 657 tests通过，30 tests失败，另有2个 failure cleanup引发的 unhandled errors。

30 个失败全部来自尚未轮到的 execution consumer：

- `tests/websocket-gateway.test.ts`：29 个 legacy connect/receive/context gateway tests；
- `tests/loop-risk-health.test.ts`：1 个 legacy receive storm test。

共同第一失败是这些 execution fixtures仍发送已删除的
`websocketConnect.contextPayloadPresent`，current validator按本 leaf要求 fail closed。更新 gateway、
dispatcher、receive queue或这些 execution fixtures属于后继 Router consumer，不在本 leaf写入范围。
根据本阶段“尚未轮到的下游允许暂时不可用”规则，没有为使全 inventory变绿而加入 dual-read、
compatibility adapter或 legacy fallback。真正应用 filename filter的相同 package script命令见
5.1，56 个 current wire tests全部通过。

## 6. Reverse search与后继边界

对 current production files执行 `receiveEvent`、`messageResponse`、`contextPayloadPresent`、
`contextCodec`、`payloadContext`、`operationAbiId`、`contractOperationId`、`websocketAdapter`、
`websocket.message` 和 `websocket.context` 反向搜索，结果为零。

implementation新增行中的这些 spelling只存在于 shared corpus 的 strict rejection mutations。F425A
冻结给后继 Runtime/Host与Router consumer的旧 general envelope/request/response types仍留在原 owner；
本 leaf没有扩张删除它们，也没有让 current exact readers接受它们。

后继 consumer需要：

- Runtime/Host owner pattern-match新的 Rust HTTP/connect wire enum并实现 connect execution；
- Router owner生成 current connect request、消费 current accept/reject并替换 legacy
  context/receive gateway路径。

本 leaf没有修改这些 owner，也没有 merge、rebase、push、stable/live/instance操作。
