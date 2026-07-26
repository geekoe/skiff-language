# P5-F385 Router test gateway control plane result

状态：Completed（R0 Router isolated HTTP test-dispatch checkpoint）。

## 1. Exact checkpoint与边界

| 项目 | commit | tree |
| --- | --- | --- |
| task base | `d8b80e5baee899f2915dd0fd01425877abd221f5` | `24269179226eda5c2829bfefbe2105b980411151` |
| production/tests | `fced15a57295417849250f686870ca1c0c395a6b` | `ab86b8656eb7dd30a07b824566ca92fdcda90fdb` |

工作分支为`codex/p5-f385-router-test-gateway-control`，worktree为
`/Users/geek/workspace/skiff-p5-f385-router-test-gateway-control`。

本任务只修改：

- `router/src/router/assemblyControlPlane.ts`
- `router/src/router/assemblyHttpGateway.ts`
- `router/src/router/assemblyRuntimeRegistry.ts`
- `router/src/router/runtimeDispatcher.ts`
- `router/tests/assembly-runtime-endpoint.test.ts`
- `router/tests/runtime-assembly-unary-dispatch.test.ts`

没有修改`router/src/protocol/**`、snapshot/loader DTO、general legacy
`router/src/router/controlPlane.ts`、WebSocket业务路由、Rust、其它仓库或stable/live状态；没有
merge、rebase或push。

## 2. Strict control与exact active match

- `decodeRuntimeAssemblyTestDispatch`只接受六个必需top-level字段：
  `kind/routing/mode/httpRequest/payloadBase64/timeoutMs`。
- `kind`必须精确为`test`。routing、mode和HTTP metadata通过现有F359 strict validator解码，
  所有nested unknown/missing field、v1 assembly、非法identity/generation及非canonical metadata
  shape均fail closed。
- control parser不修改host/method/path/identity。`exactTestDispatchBinding`逐值比较active
  assembly identity、generation、HTTP selector、gateway entry identity和mode；因此依赖
  lowercase/uppercase修复才能命中的值会被拒绝。
- payload只接受canonical standard Base64；decode后重新encode必须逐字节相等。timeout必须为正
  safe integer。
- `contractOperationId`、deployment/key、control `testEffectsEnabled`、
  `testEffectDoubles`及任意unknown field全部拒绝。

## 3. Test-only header、dispatcher与registry seam

- production `assemblyHttpRequestHeader`签名和行为不变，继续固定
  `testEffectsEnabled: false`。
- 新增内部`assemblyTestHttpRequestHeader`：
  - 先调用production builder取得全部F359 canonical facts；
  - exact比较control routing/mode与production facts；
  - 只在该专用路径把flag改为true；
  - 再次调用F359 canonical validator。
- 新增`RuntimeDispatcher.dispatchAssemblyTestBinary`，只调用registry的
  `pickAssemblyTestDispatchConnection`；没有`skipValidation`、`allowTestEffects`或通用boolean开关。
- ordinary `dispatchBinary`继续经过`validateDispatchRequest`，true仍精确报
  `active RuntimeAssembly dispatch rejects test effect controls`。
- test-only registry入口要求flag精确为true，然后与ordinary入口共享
  `validateAssemblyRequestFacts`，重复校验assembly/generation/selector/gateway identity/mode、
  adapter/stream约束与HTTP metadata；false或legacy header在socket前拒绝。
- 两个dispatcher入口只在connection selection上分流，后续共用同一个binary pending/response
  lifecycle，不复制runtime wire协议。

## 4. Response与测试矩阵

control success原样返回runtime canonical `response.end` header及payload Base64。Router不解析或
重编码业务JSON；正例以opaque `null` bytes验证request和response均逐字节不变。parse、active match、
registry validation、dispatch及runtime error继续为non-2xx control error。

direct正负矩阵覆盖：

- exact `kind:test` active request发出true F359 header；
- header routing/mode/httpRequest逐值等于control及snapshot facts；
- canonical `response.end` status/headers及payload原样回传；
- production builder false、ordinary dispatcher拒绝true；
- test-only dispatcher拒绝false和legacy header；
- old operation/doubles/control flag、deployment/key及top-level/nested unknown fields；
- wrong/missing kind、v1 assembly、stale generation、wrong identity/mode/selector；
- host/method case修复、URL/path mismatch、noncanonical Base64、zero/unsafe timeout；
- 所有control负例均证明没有`request.start`到达runtime socket。

## 5. Verification

先用`vitest list`枚举指定文件并确认全部非零：

| test file | tests |
| --- | ---: |
| `tests/assembly-runtime-endpoint.test.ts` | 12 |
| `tests/runtime-assembly-unary-dispatch.test.ts` | 15 |
| `tests/assembly-replica-dispatch.test.ts` | 1 |
| 合计 | 28 |

| 命令 / gate | 结果 |
| --- | --- |
| `pnpm --filter @skiff/router exec vitest list <三个指定文件>` | PASS；12 / 15 / 1，全部非零 |
| `pnpm --filter @skiff/router exec vitest run <三个指定文件>` | PASS；3 files / 28 tests |
| `pnpm --filter @skiff/router exec tsc --noEmit --pretty false` | 预期非零；45个错误全部为F378/F364已记录的HTTP-only/WS或相邻旧consumer残留；R0-owned HTTP/control production与direct tests零错误 |
| scoped production旧字段与通用开关反搜 | PASS |
| `git diff --check` | PASS |

最终typecheck残留逐项为：

| 范围外owner | errors | 分类 |
| --- | ---: | --- |
| `src/gateway/assemblyWebSocketGateway.ts` | 8 | 旧WebSocket binding/request DTO consumer |
| `src/router/assemblyRuntimeRegistry.ts` | 4 | 禁止修改的`canonicalAssemblyWebSocketIngressIdentity` helper |
| `tests/assembly-websocket-gateway.test.ts` | 19 | 旧WebSocket request DTO fixture |
| `tests/compilerGeneratedManifestCompatibility.test.ts` | 1 | 已记录的`globalIngress` fixture |
| `tests/loop-risk-health.test.ts` | 1 | legacy/canonical request header union |
| `tests/router-websocket-trust-dispatch.test.ts` | 5 | 旧WebSocket selector/caller fixture |
| `tests/runtime-endpoint-connection-send-trust.test.ts` | 5 | 旧WebSocket binding/contract fixture |
| `tests/service-error-cross-layer-convergence.test.ts` | 2 | HTTP/WebSocket混合旧selector fixture |
| 合计 | 45 | 本任务禁止越界修复 |

本任务移除了原基线`assemblyControlPlane.ts`的两处旧operation/test-effect builder type error；当前
`assemblyControlPlane.ts`、`assemblyHttpGateway.ts`、`runtimeDispatcher.ts`及两个changed direct
test files均无type error。`assemblyRuntimeRegistry.ts`的四处错误全在未修改的WebSocket identity
helper。

scoped production反搜范围为：

```text
assemblyControlPlane.ts
assemblyHttpGateway.ts
runtimeDispatcher.ts
assemblyRuntimeRegistry.ts中canonicalAssemblyWebSocketIngressIdentity之前的HTTP/registry部分
```

搜索集合
`ContractOperationId|contract_operation_id|contractOperationId|testEffectDoubles`为零；
`skipValidation|allowTestEffects`在四个production文件也为零。唯一production旧字段命中仍是
`assemblyRuntimeRegistry.ts`禁止修改的WebSocket helper；测试中的旧字段只作为明确reject mutation。

## 6. 自验收矩阵

| 任务条款 | 代码证据 | 反向搜索证据 | 测试 |
| --- | --- | --- | --- |
| exact six-field strict control | `assemblyControlPlane.ts:175` `decodeRuntimeAssemblyTestDispatch`复用F359 strict validator并校验Base64/timeout | control production旧字段零匹配 | endpoint exact正例；old/unknown/missing/nested/Base64/timeout负例 |
| exact active snapshot match | `assemblyControlPlane.ts:263` `exactTestDispatchBinding`逐值比较assembly/generation/selector/identity/mode | 无host/method normalization写回 | stale/wrong/case/URL/path矩阵，runtime socket保持零request |
| production false与private true builder | `assemblyHttpGateway.ts:224` production builder固定false；`:274` test builder复用后置true并重验 | 无caller boolean、无通用开关 | direct production false、test true及legacy header拒绝 |
| isolated dispatcher/registry | `runtimeDispatcher.ts:238`；`assemblyRuntimeRegistry.ts:304,553` | ordinary validation路径仍保留false拒绝 | ordinary true报冻结错误；test false失败；true成功round-trip |
| opaque canonical response | `assemblyControlPlane.ts:122-133`只投影runtime header与payload Base64 | 无业务JSON decoder | canonical response header及`null` bytes逐字节相等 |

R0 checkpoint已解除后继T1 package-test control consumer前置；本任务没有自行承接T1/T2。
