# P5-F362 Router RuntimeAssembly v2 snapshot result

状态：Completed（Router snapshot/filesystem exact join leaf；HTTP request builder、runtime registry、
WebSocket consumer仍由后续leaf迁移）。

## 1. Exact checkpoints

| 项目 | commit | tree |
| --- | --- | --- |
| integration base | `b4a03c26d9a74a1ce026d36f816020069f972535` | `833af5f87aa8a65732c2535fa7844d42f5710bac` |
| task checkout | `001054561d10f996f3a7287f484576b73eddb9e5` | `911246e79f1220aad620674e4f55a9c407e83fef` |
| production/tests | `d37418ccdb76faf2e42cafe24f9e5b6d87f93a7c` | `b0c925722c16ffe23938c2dfa55e3b43a8a2c3c0` |

工作分支为`codex/p5-f362-router-assembly-snapshot`，worktree为
`/Users/geek/workspace/skiff-p5-f362-router-assembly-snapshot`。本leaf没有merge/rebase
integration或main，没有修改lockfile、shared protocol、Rust、Host、request builder、runtime
registry/dispatcher、test-runner、stable/live配置，也没有push。

## 2. RuntimeAssembly v2 snapshot

- `runtimeAssemblySnapshot.ts`只接受`skiff-runtime-assembly-v2`与
  `skiff-runtime-assembly-v2:sha256:<lower-hex>`；memory与filesystem loader都拒绝v1。
- required ingress字段为`gatewayIngress`。每个声明严格只有`selector`、exact
  `ServiceDeploymentRef`、`gatewayEntryKey`与`gatewayEntryIdentity`；`globalIngress`、contract、
  operation和WebSocket selector均fail closed。
- selector当前严格为HTTP：artifact中的Host必须是canonical lowercase Host，method必须是required
  uppercase HTTP token，path必须是无query/fragment/whitespace/control的absolute path。Router lookup
  key仍按Host lowercase与method uppercase匹配真实HTTP请求。
- active snapshot中的每个binding只保留：

```text
selector
exact deployment
gatewayEntryKey
gatewayEntryIdentity
adapterKind
operationMode
optional timeoutMs
```

  handler/pre/guard、adapter plan、external schema、Package callable和ServiceContract operation均不进入
  snapshot或request builder。
- `resolvedContracts` exact refs仍原样保留，供internal service deployment/registration consumer使用；
  HTTP ingress不再读取ServiceContract record或按operation推导mode。
- committed、pending与request activation均从`gatewayIngress`建立replaceable index；既有environment、
  monotonic generation和same-generation no-fork规则保持。

## 3. Filesystem exact ServiceDeployment join

- filesystem loader按每个`resolvedDeployments` exact ref生成唯一canonical path：

```text
records/service-deployments/
  <encoded-service-id>/<contract-version>/<deployment-revision>/<deployment-sha256>.json
```

- service coordinate、version、revision和identity segment均先做canonical lexical validation；既有
  realpath containment、strict duplicate-key JSON和`64 MiB` bounded record读取保持。
- `ServiceDeployment`必须是strict `skiff-service-deployment-v2` top-level shape；record的contract
  service/version、revision与deployment identity逐字段等于路径来源ref。
- loader从`gatewayEntries[key]`只投影declared gateway identity、HTTP
  `adapterKind/dispatchMode`和deployment `policy.timeoutMs`。timeout缺失表示无override；present值必须是
  正safe integer。
- deployment ingress canonical union与assembly `gatewayIngress`按selector逐项exact join：
  deployment ref、selector四字段、key与entry identity任一不等即拒绝；missing/extra/duplicate
  selector、missing key、非HTTP surface、adapter kind错配、unary/stream schema错配及
  `typedJson + serverStream`均fail closed。
- join检查静态entry/plan的strict outer shape，但只返回dispatch所需的三个投影事实；不会把
  handler/pre/guard、args或schema保存到snapshot。
- 新的`runtimeAssemblyDeploymentSnapshot.ts`单独拥有deployment decode/join职责，避免让activation
  snapshot/store文件继续混入filesystem hydration实现。

## 4. Rust identity owner保持唯一

删除了Router filesystem loader中的：

- `computeRuntimeAssemblyIdentity`
- `canonicalRuntimeAssemblyIdentityValue`
- `computeServiceProtocolIdentity`
- `sha256Hex` / `stableStringify` identity依赖

Router现在只验证canonical path、strict lexical、declared exact identity和assembly/deployment交叉引用。
任意合法v2 declared assembly identity在这些条件满足时可加载；内容identity的producer/validator仍是Rust
artifact owner。HTTP ingress也不再加载`records/service-contracts/**`。

## 5. 聚焦测试证据

先用`vitest list`枚举实际非零集合：

| test file | tests |
| --- | ---: |
| `tests/active-assembly-reload.test.ts` | 6 |
| `tests/assembly-runtime-endpoint.test.ts` | 10 |
| `tests/filesystem-runtime-assembly-snapshot-loader.test.ts` | 25 |
| `tests/host-ingress.test.ts` | 8 |
| 合计 | 49 |

聚焦运行结果为`4 files / 49 tests PASS`。覆盖：

- raw HTTP unary、raw HTTP server stream、typed JSON unary；
- exact assembly/deployment join与canonical record path；
- deployment timeout override及无override；
- declared identity不在TypeScript重算；
- v1 schema/prefix、`globalIngress`、contract operation和WebSocket selector拒绝；
- wrong key/identity/ref coordinate/revision、missing/extra/duplicate selector与missing key；
- non-HTTP protocol、adapter kind/mode/schema错配、typed JSON stream、zero/null timeout；
- snapshot只保留dispatch投影字段；
- activation prepare/commit/abort/recovery、replace和generation/no-fork规则。

## 6. Type-check边界与剩余consumer

`pnpm --filter @skiff/router exec tsc --noEmit --pretty false`按任务预期exit 1。共81个错误，owned
production与本leaf四个test file均为零错误；全部命中尚未迁移或本任务禁止修改的下游consumer/tests：

| consumer | errors | 剩余旧消费 |
| --- | ---: | --- |
| `src/router/assemblyRuntimeRegistry.ts` | 17 | snapshot contract/operation及F359前request/WebSocket字段 |
| `src/router/runtimeDispatcher.ts` | 18 | F359前WebSocket/operation request字段 |
| `src/gateway/assemblyWebSocketGateway.ts` | 8 | WebSocket selector、contract/operation旧snapshot模型 |
| `src/router/assemblyHttpGateway.ts` | 1 | HTTP builder仍读取`contractOperationId` |
| `src/router/assemblyControlPlane.ts` | 1 | control projection仍读取`contractOperationId` |
| `tests/assembly-websocket-gateway.test.ts` | 19 | WebSocket旧snapshot/request fixture |
| `tests/runtime-endpoint-connection-send-trust.test.ts` | 5 | WebSocket旧snapshot fixture |
| `tests/router-websocket-trust-dispatch.test.ts` | 5 | WebSocket旧snapshot/request fixture |
| `tests/service-error-cross-layer-convergence.test.ts` | 2 | HTTP/WebSocket混合旧selector fixture |
| `tests/assembly-http-gateway-stream.test.ts` | 1 | HTTP旧contract fixture |
| `tests/assembly-replica-dispatch.test.ts` | 1 | HTTP旧contract fixture |
| `tests/runtime-assembly-unary-dispatch.test.ts` | 1 | HTTP旧contract fixture |
| `tests/compilerGeneratedManifestCompatibility.test.ts` | 1 | C4 authoring fixture仍断言`globalIngress` |
| `tests/loop-risk-health.test.ts` | 1 | F359 legacy/canonical request header union |

这些断点没有通过兼容字段、dual reader或跨owner修改掩盖。尤其没有为WebSocket设计新的业务消息、entry
identity或selector模型。

## 7. 验证

| 命令 / gate | 结果 |
| --- | --- |
| 四个test file的`vitest list` | PASS；6 / 10 / 25 / 8，均非零 |
| 四个test file的`vitest run` | PASS；4 files / 49 tests |
| Router full `tsc --noEmit --pretty false` | 预期exit 1；81个全在上述downstream，owned零错误 |
| owned production legacy/identity反搜 | PASS；零匹配 |
| `git diff --check` | PASS |

owned production反搜集合为：

```text
globalIngress
contractOperationId
skiff-runtime-assembly-v1
computeRuntimeAssemblyIdentity
computeServiceProtocolIdentity
```

filesystem/deployment snapshot上的`ServiceContract|service-contracts|sha256Hex|stableStringify`反搜同样
为零。未运行root/workspace、stable/live或网络服务验证。

## 8. 自验收矩阵

| 任务条款 | 代码证据 | 测试/验证证据 |
| --- | --- | --- |
| strict v2 + HTTP-only | `runtimeAssemblySnapshot.ts` v2 record/selector decoder | v1/global/operation/WebSocket negatives |
| canonical deployment path | `loadServiceDeployment` coordinate/version/revision/identity path | exact path load、missing/mismatch、symlink escape |
| exact deployment join | `runtimeAssemblyDeploymentSnapshot.ts::joinRuntimeAssemblyDeployments` | wrong ref/key/identity、missing/extra/duplicate selector/key |
| HTTP surface/mode/policy | strict gateway surface与policy projection | raw unary/stream、typed unary、mode/schema/timeout negatives |
| minimal snapshot | `RuntimeAssemblyIngressBinding` final field set | exact key-set assertion，无handler/plan/operation |
| no ServiceContract mode lookup | loader只读deployment，`resolvedContracts`只透传 | 有resolved contract而无contract record仍成功 |
| Rust identity single owner | TS hash projection全部删除 | declared v2 identity非重算正例与反搜 |
| activation regression | committed/request/pending改读`gatewayIngress` | 16个activation/endpoint tests PASS |
| downstream containment | 禁止范围零修改 | full type-check断点精确归档，owned零错误 |
