# P5-F445H-D2 WebSocket peer-cancel hard cut result

状态：`PASS / IMPLEMENTED`。

Implementation commit：`bb304998`（`router: remove websocket peer cancellation protocol`）。

## 1. 实现结果

- JSON-RPC 2.0 text profile删除`cancelled` platform error、`cancel` action、
  `encodeCancel`、cancel-specific params parser与`-32800 Request cancelled`编码。
- 所有合法notification统一分类为`ignoredNotification`；`$/cancelRequest`没有method-name特例。
  带`id`的同名frame继续作为普通request，按captured method table判断是否dispatch。
- Broker删除inbound peer cancel和outbound peer cancel投影。Runtime内部cancel、outbound deadline与
  runtime source disconnect都先收束本地pending并写入既有bounded tombstone，不再增加peer writer
  计数。
- Notification只进入诊断observer；unknown、active或already-settled id都不会dispatch、response或
  abort handler，observer异常仍被隔离。
- Router与Runtime README现在明确所有peer notification均被忽略；broker继续拥有deadline、
  Runtime内部stop和settled-state fencing，内部stop不是peer cancellation。
- Runtime/Rust production、runtime transport schema、`RuntimeEndpoint`、`RuntimeDispatcher`、
  `WebSocketRpcBridge` production、gateway、durable queue、Actor与spawn均未修改。

## 2. RED与聚焦验证

先把合法、无`params`的`$/cancelRequest` notification测试改成
`ignoredNotification`，production修改前实际执行：

```text
tests/json-rpc-20-text-profile.test.ts: 35 tests
34 passed / 1 failed
```

失败是旧implementation返回`platformError.invalidRequest`，与新期望形成真实RED。

最终验证：

```text
router/node_modules/.bin/vitest list --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/websocket-rpc-bridge.test.ts
```

列出`98`个测试，非零。

```text
router/node_modules/.bin/vitest run --root router \
  tests/json-rpc-20-text-profile.test.ts \
  tests/websocket-request-broker.test.ts \
  tests/websocket-rpc-bridge.test.ts
```

结果：`3`个test files通过，`98/98` tests通过。

```text
pnpm --dir router type-check
git diff --check
```

两项均通过。Worktree原本没有依赖；验证仅临时链接主仓库已有
`router/node_modules`、root依赖所需的现有MongoDB package，验证后已全部删除。没有install、network、
stable/live、长期server或阶段完整gate。

## 3. 反向搜索

- Router production与`router/README.md`、`runtime/README.md`中的`$/cancelRequest`、`-32800`、
  `Request cancelled`均为`0`。
- Profile contract/implementation中的`cancelled` error、`ProfileAction.kind = "cancel"`与
  `encodeCancel`均为`0`。
- Broker production中的`cancelPeer`、`bestEffortCancel`、`tryEncodePeerCancelFrame`与
  `handleInboundCancel`均为`0`。
- 三个聚焦测试中的`$/cancelRequest`只作为普通带`id` request或被忽略notification的负例。
- `request.cancel`、`connection.request.cancel`与`RequestCancelReason`仍存在于内部transport owner；
  没有删除或放宽其strict schema。

## 4. 自验收矩阵

| 合同条款 | 代码/测试证据 | 结果 |
| --- | --- | --- |
| Profile hard cut | contracts与implementation删除三个public cancel分支；profile测试覆盖缺失/object/畸形params和带`id`普通request | PASS |
| Broker hard cut | broker/wire删除peer cancel收发；runtime cancel、deadline、source disconnect测试断言writer不增加 | PASS |
| Notification无状态影响 | broker覆盖active、unknown、settled id、observer抛错；handler最终只写normal result | PASS |
| 双向同值id隔离 | broker与bridge测试保留独立inbound/outbound pending并各自正常settle | PASS |
| Bridge真实路径 | bridge覆盖runtime cancel、deadline、source disconnect与peer notification；既有cleanup测试覆盖generation/pending/timer/tombstone归零 | PASS |
| 既有失败与晚到隔离 | socket disconnect、deadline、capacity、protocol error、late completion与tombstone测试全部通过 | PASS |
| Wire不变范围 | raw send与普通JSON-RPC request/response既有测试未改production路径且聚焦套件全绿 | PASS |
| README | 两份README删除peer cancel例外并记录internal stop仅本地收束 | PASS |
| 禁止范围 | Rust/runtime production、bridge/gateway production、schema、lockfile均无diff | PASS |

## 5. 未决事项

无实现 blocker或范围扩张。按任务合同，本提交仍是实现检查点；后续稳定候选需要由独立验收owner复核
profile → broker → bridge真实路径。
