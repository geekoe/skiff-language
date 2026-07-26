# P5-F394 Router runtime protocol current service identity result

状态：Completed。Router shared runtime protocol、canonical frame fixtures及其direct
activation/connection/dispatch tests已原子切到current
`skiff-service-protocol-v4`；well-formed v3现在fail closed，没有dual-read、转换或fallback。

## 1. Checkpoint与边界

| 项目 | commit | tree |
| --- | --- | --- |
| task base | `108f507241ad572cd20fd5444a3dc304d76ff242` | `e5710df5bfee3479e69c501b8a7dc7ec38c94de8` |
| task delivery | 本result所在本地commit | 本result所在本地commit的tree |

工作分支为`codex/p5-f394-router-runtime-protocol-v4`，worktree为
`/Users/geek/workspace/skiff-p5-f394-router-runtime-protocol-v4`。

production只修改`router/src/protocol/runtimeProtocol.ts`；测试只修改其direct protocol、
activation、connection与request/control dispatch tests。本任务没有修改protocol字段shape、
HTTP/WebSocket gateway语义、Host/runtime Rust、test-runner、snapshot/filesystem owner、其它仓库或
stable/live状态；没有merge、rebase或push。

## 2. Current runtime protocol终态

- `SERVICE_PROTOCOL_IDENTITY_PATTERN`只接受
  `skiff-service-protocol-v4:sha256:<64 lowercase hex>`。
- `runtime.register`、retained legacy `request.start`、`spawn.submit.request`、
  `spawn.claim.request`与`spawn.claim.response.item`共用同一v4-only pattern和v4错误golden。
- `runtimeFrameHeaderFixtures`的canonical registration、request与spawn identity都来自current v4
  fixture；registration直接复用该fixture常量，避免同文件内两份generation source漂移。
- exact v4 registration object通过parser；相同64位lowercase digest的well-formed v3 registration
  被明确拒绝。request/spawn三面还分别覆盖well-formed v3、坏长度与大写digest负例。

没有协议字段或frame schema版本变化，因此没有触发`TASK_SCOPE_EXPANDED`。

## 3. Exact identity传递

- `assembly-runtime-endpoint.test.ts`用v4 contract snapshot完成真实coordinator initialization、
  activation registration与同一socket上的spawn submit/claim。claim response item断言
  `serviceProtocolIdentity`逐值等于assembly中的exact v4 identity。
- `assembly-replica-dispatch.test.ts`把同一v4 identity放入active snapshot；runtime connection
  registration后，`actorSpawnRuntimeControlSource`返回的identity与snapshot exact value相同，证明
  connection binding没有重算identity。相邻healthy replica request dispatch仍通过。
- `runtime-registry-dispatch.test.ts`的direct runtime registration/request fixtures切到v4，并保留
  same-target不同protocol/build、current build protocol mismatch等边界测试。
- `actor-spawn-runtime-control.test.ts`和`router-default-spawn-probe.test.ts`的direct registration、
  activation与control frames均切到v4。后者已有的assembly v1测试常量在首轮执行时先被current
  activation lexical gate拒绝；该direct fixture同步为仓库既有assembly v2后，原两个case均通过。

Router没有新增identity计算或转换代码；除shared lexical gate外没有production写入。

## 4. 反向搜索与范围外owner

`router/src/protocol/runtimeProtocol.ts`内
`skiff-service-protocol-v3`为零。六个本任务direct test文件中的三个v3命中全部是
`protocol.test.ts`的显式拒绝负例；正例、canonical fixtures与错误golden均为v4。

Router其它production仍有两个明确独立owner，本任务按写入边界没有迁移：

- `router/src/manifest/loadManifest.ts`：legacy runtime manifest parser/identity authoring；
- `router/src/artifacts/identity.ts`：legacy artifact identity authoring。

它们的direct/archived fixtures分布在`router/fixtures/hello/manifest.json`、
`tests/{actor-production-routing,artifact-reload,artifacts,identity,manifest-validation,raw-http,release-routing,test-dispatch,websocket-gateway}.test.ts`
及`tests/helpers/{manifests,websocketFixtures}.ts`，应随上述manifest/artifact owner迁移。

另有current assembly WebSocket下游fixture命中位于
`tests/{assembly-websocket-gateway,assembly-websocket-ingress-identity,router-websocket-trust-dispatch,runtime-endpoint-connection-send-trust,service-error-cross-layer-convergence}.test.ts`。
这些测试仍绑定父结果已登记的旧WebSocket binding/request DTO，属于明确的WebSocket owner；本任务禁止
改变其字段或gateway语义。

## 5. Verification

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| focused non-zero tests | `pnpm --filter @skiff/router exec vitest run tests/protocol.test.ts tests/assembly-runtime-endpoint.test.ts tests/assembly-replica-dispatch.test.ts tests/actor-spawn-runtime-control.test.ts tests/router-default-spawn-probe.test.ts tests/runtime-registry-dispatch.test.ts` | PASS；6 files / 111 tests |
| protocol scoped type-check | `pnpm --dir router exec tsc --noEmit --pretty false --target ES2022 --lib ES2022 --module NodeNext --moduleResolution NodeNext --strict --noUncheckedIndexedAccess --exactOptionalPropertyTypes --esModuleInterop --skipLibCheck --forceConsistentCasingInFileNames src/protocol/runtimeProtocol.ts tests/protocol.test.ts` | PASS |
| Router type-check audit | `pnpm --filter @skiff/router type-check` | 预期非零；精确44个错误，与F392父结果的文件分类和数量完全相同；本任务owner/direct parser test零错误 |
| production旧generation反搜 | `runtimeProtocol.ts`内搜索`skiff-service-protocol-v3` | PASS；零匹配 |
| direct-test旧generation反搜 | 六个direct tests内搜索v3 | PASS；仅3个显式拒绝负例 |
| patch hygiene | `git diff --check` | PASS |

Router全局type-check的既有44个错误仍精确分布为：

| 范围外owner | errors |
| --- | ---: |
| `src/gateway/assemblyWebSocketGateway.ts` | 8 |
| `src/router/assemblyRuntimeRegistry.ts` | 4 |
| `tests/assembly-websocket-gateway.test.ts` | 19 |
| `tests/loop-risk-health.test.ts` | 1 |
| `tests/router-websocket-trust-dispatch.test.ts` | 5 |
| `tests/runtime-endpoint-connection-send-trust.test.ts` | 5 |
| `tests/service-error-cross-layer-convergence.test.ts` | 2 |
| 合计 | 44 |

worktree起初没有安装Router dependencies；执行
`pnpm --dir router install --frozen-lockfile`后没有修改tracked dependency文件。

## 6. 自验收矩阵

| 任务条款 | 代码/测试证据 | 反向搜索 | 验证 |
| --- | --- | --- | --- |
| runtime protocol v4 only | shared pattern、五个validator message、canonical fixtures | production owner无v3 | protocol parser正负例；111/111 |
| 所有direct fixtures/current goldens | protocol、activation、connection、spawn与registry dispatch tests | direct正例无v3 | 6 files / 111 tests |
| activation/registration/dispatch exact传递 | assembly endpoint claim item；replica registered control source；registry request dispatch | 无转换、fallback或identity recompute production改动 | exact v4 assertions PASS |
| v3 fail closed | well-formed v3 registration/request/spawn负例 | v3只保留在负例 | parser test PASS |
| shape/语义/边界闭合 | production仅`runtimeProtocol.ts` lexical generation；其余均direct tests/result | 范围外owner逐项报告 | scoped tsc PASS；global仅既有44；diff check PASS |
