# P5-F433A Current WebSocket fixture and oracle convergence result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED`。

## 1. 提交与范围

implementation：

- commit：`c0068fdc6bf1fcffd019b79263228c429796be67`
- tree：`a4ebd79f4586fcad11997137805bd3519c1f8155`

result-only commit由交付消息记录。

本 leaf 只修改任务授权的四个 fixture、`package_service_contract_deployment.rs`和直接
`package-service-*.test.mjs`，并新增本 result。没有修改 test-runner production、
Runtime/Router/compiler/std、Internals或skiff-packages；没有运行或操作 stable/live/instance，
没有承接后继 combined probe，也没有 merge、rebase或 push。

## 2. Fixture终态

四个 fixture 现在都是显式 `kind: test` service，并带可执行的
`config.skiff-test.yml` profile。每份 `service.yml`只有一个 strict singleton WebSocket entry：

```yaml
websocket:
  path: /socket
  connect:
    handler: main.websocketConnect
    adapterArgs:
      - param: request
        source: { kind: websocket.connectRequest }
      - param: connectionId
        source: { kind: websocket.connectionId }
```

private `main.websocketConnect`的签名统一为：

```text
(std.websocket.WebSocketConnectRequest, connectionId: string)
  -> std.websocket.WebSocketConnectResult
```

accept只把`connectionId`用作`businessIdentity`并保留可选 policy；没有 Context。`api.yml`
不导出 connect callable，只保留三个 marker surface和I02的
`marker: main.submitSpawnReceipt`。旧 unified ingress、receive branch、client-message echo和
connection send均已删除；I02仅为 echo 存在的旧 marker helper也一并删除。

inline `ecosystem_http_fixture_uses_two_gateway_entries_without_ws_compat` package已删除
`import std`、旧WebSocket API与全部WebSocket source，没有伪造`service.yml`。它仍生成精确两个
HTTP gateway entry、零service operation；因不再依赖 std，assembly resolved package数从3变为2。

## 3. 真实生成与identity oracle

`ecosystem_http_private_wrappers_compile_for_all_owned_source_fixtures`现在通过真实
`compile_service_package -> generate_service_deployment -> resolve_runtime_assembly`链生成四份
current service artifact，并验证：

- contract与deployment operation均为零；
- 恰好一个`websocket` gateway entry和一个`/socket` WebSocket ingress；
- handler精确join到private `main.websocketConnect` callable；
- adapter参数顺序和两种source精确；
- gateway protocol identity固定为
  `skiff-gateway-entry-v1:sha256:d32884370c32e2a3923cbc7245d30c5a56c68b272825cde3645a1a48b49a5936`。

fresh current identity oracle如下；没有保留或伪造旧source digest：

| fixture | package build | local ABI | generated deployment | single-root assembly |
| --- | --- | --- | --- | --- |
| websocket-smoke | `5ce089038445f6ea1bf05a5d8876ebb784c9193f4509ee993f0eb6b415c25880` | `d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba` | `3e020a778a528ff61ddb4b953186299b9145beaa7c368bb8fa121a8c7db8ccf5` | `d949679862b2e0b5cff67cbd517bab56b6bb7b2165906a3860811b3db181c342` |
| generation-a | `25b98b03e66c0a7398859a6d0362dd53c20ff39f77ea36377408b74da6bfb37b` | `d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba` | `6a4e17954474836b8a2511442e44855b16a0d51d77e4b82fd90d8842daf9c9c5` | `6cae8bf053cba5247aafe4ef4ab635d453cd9688f6935ad01891679d6ed3f1dd` |
| generation-b | `3f42eb72f997ccf6a65b986cb49af485ae63c67db32e5b587822cfadf9c5e791` | `d5627a25f7edd95d81505910f4d86f89434f2eff3837475ebf9e2b31f257b9ba` | `0897f23f6972709688cf420e30589ff2cb64380cd63a4e45cc6938aa96308d8b` | `f73298e2908b53e535d1fe4f6b7c654166a304c1f0468b0c88a8feba08c4f079` |
| I02 spawn-submit | `6f686ba330266ad08baf8dd04baba0bfcc315ec4e0ed8308344f9f8a7f8230b8` | `3db7056f815676834489b34a069b5016f05973b3be9379eb55736a545d7dcdf9` | `6eb0fffd40ee1d373db397063ae81e587ec564852be149bfa1f225bc763c8766` | `11b8f9d38d44c642438f37a6b787b58b3deca8545d9c48d850a6a0c00813752a` |

表中省略的canonical prefix依次是`skiff-package-build-v10:sha256:`、
`skiff-package-local-abi-v7:sha256:`、`skiff-deployment-artifact-v2:sha256:`和
`skiff-runtime-assembly-v2:sha256:`。三个marker fixture的ABI相同是预期结果：它们公开及private
callable signature相同；marker body不同仍使package build、deployment和assembly identity彼此不同。

## 4. 验证与discovery

| 命令 | 结果 |
| --- | --- |
| 两条任务指定Rust filter | 各实际执行1项，`1 passed / 23 filtered out` |
| 额外current authoring/compiler/deployment/assembly identity filter | `1 passed / 23 filtered out` |
| `node --test scripts/tests/skiff-source-test-suite.test.mjs` | `10 / 10 PASS` |
| `node --test scripts/tests/package-service-i02-combined.test.mjs` | `6 / 6 PASS` |
| `node --test scripts/tests/package-service-*.test.mjs` | 实际discovery `38 / 38 PASS`；包含上述I02 6项及新增ecosystem fixture assertion |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |

completion反搜对四fixture、所属Rust测试和两份直接script assertion检查
`WebSocketIngressEvent`、generic `WebSocketConnectResult<`、`receiveEvent`、
旧send/context/API/handler spelling，结果为0。四个fixture分别正向命中一次
`std.websocket.WebSocketConnectRequest` source，以及一份同时含
`websocket.connectRequest`和`websocket.connectionId`的strict service authoring。

implementation提交后工作树clean；result提交后的最终clean状态由交付消息记录。
