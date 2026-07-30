# P5-F364 Router HTTP gateway dispatch result

状态：Completed（C3 Router HTTP canonical dispatch leaf；RuntimeAssembly WebSocket业务入口继续
fail closed）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `b71e622ca35109519e904f269a67f19bc2f08de4` | `7d79c140534db0ed2336e3babe511fa444fdc2e6` |
| task checkout | `dff07b75dadabb5ebd624302634239ce728bf547` | `6e98e67e227004a1f3c63c4afb0e1005006b2cb9` |
| production/tests | `d11a3dc2fe4330b6beae273f4ad31619e84a17b3` | `ba028a9535a9f060c6e0786b3f406a7ff0babc83` |

工作分支为`codex/p5-f364-router-http-dispatch`，worktree为
`/Users/geek/workspace/skiff-p5-f364-router-http-dispatch`。本leaf没有merge/rebase integration或
main，没有修改F359/F362 canonical DTO、protocol owner、snapshot/loader、WebSocket gateway、
control-plane、lockfile或stable/live配置，也没有push。

## 2. Canonical HTTP request与exact registry admission

- `assemblyHttpRequestHeader`只生成F359 HTTP header：
  - `caller`精确为`{ kind: "gateway" }`；
  - routing只含RuntimeAssembly identity/generation、nested
    `gatewayEntryIdentity`与HTTP ingress；
  - mode直接来自committed snapshot binding；
  - `httpRequest`保存method、URL、path、query与headers，binary frame payload保持原始body bytes；
  - 不再接受或生成caller target、contract operation、handler/adapter、top-level gateway identity、
    test double或WebSocket字段；production HTTP固定`testEffectsEnabled: false`。
- `AssemblyRuntimeRegistry`先走F359 strict validator，再逐值核验：
  - committed assembly identity与generation；
  - canonical host/method/path selector及binding中的精确selector；
  - nested gateway entry identity；
  - binding operation mode；
  - HTTP metadata method/path，以及URL的HTTP protocol、host、path和无credential/hash约束。
- registry不再从ingress恢复ServiceContract binding，也不加载或比较contract operation。actor/spawn的
  普通service binding继续只由snapshot的`resolvedDeployments + resolvedContracts`形成。
- `typedJson + serverStream`在registry和HTTP gateway两层都fail closed；唯一server-stream来源仍是
  `rawHttp` binding。

## 3. Timeout、unary与stream response

- 平台HTTP cap为显式`requestTimeoutMs`或默认`120000`；effective timeout精确为：

  ```text
  min(platform HTTP cap, deployment timeoutMs when present)
  ```

- 平台cap和deployment override都必须是正safe integer，且可由JavaScript deadline与Node timer精确
  承载；零、负数、小数、unsafe integer及超过Node timer上限的值直接失败，不回退到更长值。
- 同一个effective值写入`deadline.timeoutMs`/`expiresAt`并传给dispatcher timer。server stream仍保留
  既有长流语义：该timer约束等待`response.start`，start后由end/error、client disconnect、
  backpressure/drain timeout、runtime disconnect与cancel闭合。
- unary response必须携带runtime投影的HTTP status/headers；Router在response byte ceiling通过后逐值
  写回status、headers和opaque body。rawHttp body与typedJson body都不在Router解码或重编码。
- server stream严格要求一次start、连续`seq` chunk、最后空end：
  - start必须携带HTTP status/headers且payload为空；
  - chunk必须在start后按从0开始的连续seq到达；
  - end必须在start后到达，且没有payload、HTTP或WebSocket response metadata；
  - duplicate/out-of-order frame、oversize、writer callback、backpressure、disconnect与timeout都经
    single-terminal路径发送至多一次cancel并清理pending/writer counters。

## 4. RuntimeAssembly WebSocket fail-closed

- `assemblyRuntimeRegistry.ts`删除旧WebSocket identity合成、adapter/entry校验及
  ServiceContract-derived ingress分支。
- `runtimeDispatcher.ts`删除把canonical RuntimeAssembly request解释为
  `websocketConnect/websocketReceive` phase、receipt binding和response DTO的全部分支。
- generation acquire attribution明确返回false；connection receipt重用明确拒绝，直到WebSocket业务
  message routing冻结。
- 普通legacy request dispatch、generic receipt sender identity、runtime connection lifecycle、
  connection.send及普通unary/server-stream pending逻辑继续保留。额外legacy回归108项通过。

## 5. Verification

先枚举指定test files并确认全部非零：

| test file | tests |
| --- | ---: |
| `tests/assembly-http-gateway-stream.test.ts` | 7 |
| `tests/assembly-replica-dispatch.test.ts` | 1 |
| `tests/runtime-assembly-unary-dispatch.test.ts` | 14 |
| 合计 | 22 |

| 命令 / gate | 结果 |
| --- | --- |
| 指定三个test file的`vitest list` | PASS；7 / 1 / 14，均非零 |
| 指定三个test file的`vitest run` | PASS；3 files / 22 tests |
| `raw-http.test.ts`、`runtime-registry-dispatch.test.ts`、`websocket-gateway.test.ts` | PASS；3 files / 108 tests |
| Router full `tsc --noEmit --pretty false` | 预期非零；pnpm wrapper exit 1、tsc child exit 2；46个错误全部在禁止consumer，owned production/direct tests为零 |
| owned production legacy反搜 | PASS；零匹配 |
| `git diff --check` | PASS |

完整type-check剩余错误分类：

| 禁止consumer | errors | 分类 |
| --- | ---: | --- |
| `src/gateway/assemblyWebSocketGateway.ts` | 9 | 未冻结RuntimeAssembly WebSocket selector/identity/contract旧消费 |
| `src/router/assemblyControlPlane.ts` | 2 | 旧contract operation与test-effect HTTP builder参数 |
| `tests/assembly-websocket-gateway.test.ts` | 19 | 旧WebSocket request DTO fixture |
| `tests/compilerGeneratedManifestCompatibility.test.ts` | 1 | C4前`globalIngress` fixture |
| `tests/loop-risk-health.test.ts` | 1 | legacy/canonical request header union |
| `tests/router-websocket-trust-dispatch.test.ts` | 6 | 旧WebSocket selector/caller/identity fixture |
| `tests/runtime-endpoint-connection-send-trust.test.ts` | 6 | 旧WebSocket binding/contract fixture |
| `tests/service-error-cross-layer-convergence.test.ts` | 2 | HTTP/WebSocket混合旧selector fixture |
| 合计 | 46 | owned production与本leaf三个direct test files均无错误 |

反向搜索范围为：

```text
router/src/router/assemblyHttpGateway.ts
router/src/router/assemblyRuntimeRegistry.ts
router/src/router/runtimeDispatcher.ts
router/src/router/httpStreamResponseWriter.ts
```

搜索集合：

```text
contractOperationId|callerTarget|testEffectDoubles|websocketAdapter|websocketEntryId
```

结果为零。未运行root/workspace完整gate或任何stable/live验证。

## 6. 自验收矩阵

| 任务条款 | 代码证据 | 测试/反向证据 |
| --- | --- | --- |
| F359 exact HTTP header | `assemblyHttpRequestHeader`只组装nested gateway routing与opaque HTTP payload metadata | exact caller/routing key-set、v2 identity、非UTF-8 body正例；旧flat/operation/adapter字段负例 |
| exact committed lookup | `validateAssemblyRequest`逐值比较assembly、generation、selector、gateway identity和mode | wrong assembly/generation/host/method/path/identity/mode均在socket前拒绝 |
| effective timeout | `effectiveHttpRequestTimeoutMs`唯一计算platform/deployment最小值并严格校验 | override更小、无override、platform更小及零/负/小数/unsafe/overflow负例 |
| raw/typed unary | HTTP response metadata required；payload只按bytes写回 | raw status/header/body、typed opaque response、request/response ceiling |
| raw server stream | dispatcher sequencing + `HttpStreamResponseWriter`队列/backpressure owner | ordered binary chunks；start/chunk/end顺序、payload/metadata、oversize、timeout、disconnect、drain负例 |
| single terminal/cancel | dispatcher pending terminal与writer idempotent terminal callback | late terminal忽略、callback/oversize/backpressure/disconnect/timeout后pending与writer counters归零 |
| WebSocket fail closed | registry旧identity分支删除；dispatcher phase/receipt DTO删除并显式拒绝receipt reuse | owned WebSocket旧字段反搜为零；禁止WebSocket consumers保留明确type-check断点 |
| legacy containment | ordinary registry/dispatcher与connection lifecycle未替换 | 108项raw HTTP、legacy dispatcher、ordinary WebSocket regression PASS |
