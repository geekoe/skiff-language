# P5-F440Z1 Router WebSocket RPC current snapshot / method table result

状态：`PASS / CURRENT_READER_AND_IMMUTABLE_METHOD_JOIN_COMPLETE`。

本 leaf 已把 Router production snapshot reader hard cut 到
`ServiceDeployment v3`、`GatewayEntry v2`、`DeploymentArtifact v3`，并把同一 deployment
内的 physical WebSocket entry 与 method-bearing `websocketJsonRpc` entries 严格连接成
physical attach binding 上的 copy-on-capture method table。Gateway、socket lifecycle、broker、
RuntimeEndpoint、RuntimeDispatcher 与 server 均未修改。

## 1. 基线、提交与写集

| 项目 | Commit / Tree |
| --- | --- |
| 任务声明 implementation baseline | `85ff1513` |
| worktree 实际起点 | `07e0c25db4c80a79f73c0d3fd495884733865bf1` |
| implementation | `fe5abef86885a23bd1a84b52c040bc60729b8d3b` |
| implementation tree | `bb2b5bce27b75e89bbbbc55a9043a765cd0eff5c` |
| result | 本文独立提交；最终 commit/tree 由交付消息记录 |

Worktree：
`/Users/geek/workspace/skiff-p5-f440z1-router-rpc-snapshot`

Branch：
`codex/p5-f440z1-router-rpc-snapshot`

Implementation 只修改任务授权 reader/tests，并新增两个 reader-private owner：

- `runtimeAssemblyDeploymentIdentity.ts`：current GatewayEntry /
  DeploymentArtifact canonical preimage；
- `runtimeAssemblyWebSocketSnapshot.ts`：current WebSocket surface decoder、
  physical/method binding 与 immutable capture table。

没有修改 artifact producer/schema、Rust、Gateway、broker、dispatcher、Endpoint、server、
cross-system fixture、README/checker或其它 task/result。

## 2. Test-first RED

production 修改前先新增真实 current-shape positive：

```text
router/node_modules/.bin/vitest run --root router \
  tests/runtime-assembly-websocket-rpc-snapshot.test.ts
```

实际执行 `1` test，`1 failed`：

```text
RouterSnapshot.serviceDeployments[0].schemaVersion must be
skiff-service-deployment-v2
```

fixture 已包含 physical `websocketConnect`、closed `rpcProfiles`、method-bearing
`websocketJsonRpc`、GatewayEntry v2 与 DeploymentArtifact v3；因此 RED 精确命中旧 production
reader，而不是零测试、synthetic probe或依赖失败。

## 3. Current hard cut

production reader 现在只接受：

- `skiff-service-deployment-v3`；
- `skiff-gateway-entry-v2:sha256:<lowercase-hex>`；
- `skiff-deployment-artifact-v3:sha256:<lowercase-hex>`；
- current package build v10 / local ABI v7 implementation reference；
- RuntimeAssembly v2 与 ServiceProtocol v5 保持既有 generation。

reader 使用与 Rust owner 相同的 current canonical preimage：

- GatewayEntry：`skiff-gateway-entry-identity-v2` + exact protocol surface；
- DeploymentArtifact：`skiff-deployment-artifact-identity-v3`，排除 declared identity /
  diagnostic text，递归移除 human version labels，并按 current collection ordering计算；
- physical `WebSocketEntryId` 继续只由 service owner与 compiler-owned key
  `websocket` 导出。

filesystem loader 只解析 deployment-artifact v3 canonical path。旧 v2/v1 generation没有 alias、
dual reader、字段猜测或 prefix fallback。真实 compiler-generated HTTP artifact test同时证明
TypeScript reader与 current Rust producer preimage一致。

## 4. Physical / method strict join

### 4.1 Physical attach binding

physical entry 必须：

- selector 为 `protocol=webSocket`、`method=null`；
- compiler-owned gateway key精确为 `websocket`；
- surface / adapter plan精确为 `websocketConnect`；
- shapes均为 v1；
- external sources精确为 `connectRequest,connectionId`；
- downlink frames精确为 `binary,text`；
- `rpcProfiles`精确为 closed `jsonrpc-2.0-text`；
- `pre` / `guard` 为 null；
- connect handler可缺席；存在时必须是 current implementation package 的 private callable。

physical selector仍是唯一 attach route。`RuntimeAssemblyIngressIndex` runtime gate明确拒绝
method-bearing WebSocket binding成为 attach ingress。

### 4.2 Method table

method entry 必须：

- selector method为 non-empty string；
- surface / adapter plan精确为 `websocketJsonRpc` / unary；
- profile属于 physical closed profiles；
- host/path、deployment owner与 derived physical `WebSocketEntryId`完全一致；
- handler必填，且属于 current implementation package 的 private callable；
- external sources canonical、无重复、包含 `websocket.jsonRpcParams`，并与 adapter args source
  集合精确一致；
- params schema只接受 top-level record/array或只由二者构成的 canonical closed union；
- result schema使用 current closed external-schema vocabulary；
- `pre` / `guard` 为 null；
- declared GatewayEntryIdentity与 exact current surface preimage一致。

fail-closed direct matrix覆盖 duplicate method、orphan/missing physical、host/path drift、
cross-deployment owner、foreign physical/method handler、unsupported profile、missing handler、
wrong adapter、ambiguous physical selectors、method identity drift、surface drift与 stale generation。

RuntimeAssembly declarations仍精确包含 physical与method selectors；loaded attach ingress只返回
physical/HTTP bindings，method declarations不会形成第二条 route。

## 5. Immutable capture API

每个 physical binding持有：

```text
websocketEntryId
websocketRpcProfiles
RuntimeAssemblyWebSocketMethodTable
```

`RuntimeAssemblyWebSocketMethodTable`：

- constructor复制并冻结 deployment/method binding；
- 不暴露 internal Map；
- `capture()` 每次返回独立 `ReadonlyMap` copy；
- caller即使强制修改自己的 captured Map，也不能改变 active snapshot或其它 capture；
- active snapshot replacement后，旧 capture继续保留旧 method/handler/identity；
- memory loader clone显式保留 table owner，而不是把 class/private Map降级为 mutable/plain object。

direct test以 generation 1 `status` capture、generation 2 `acknowledge` replacement证明旧连接所需表
不再依赖 current snapshot。

## 6. 规定验证

最终 implementation tree 上：

| 命令 | 结果 |
| --- | --- |
| 规定三文件 direct Vitest listing | PASS，`61` non-zero tests |
| 规定三文件 direct Vitest run | PASS，3 files，`61/61` |
| hard-cut 五个 direct files listing | PASS，`25` non-zero tests |
| hard-cut 五个 direct files run | PASS，5 files，`25/25` |
| `pnpm --dir router type-check` | PASS |
| `git diff --check` / staged diff check | PASS |

三文件细分：

- `filesystem-runtime-assembly-snapshot-loader.test.ts`：`39`；
- `runtime-assembly-websocket-rpc-snapshot.test.ts`：`21`；
- `compilerGeneratedManifestCompatibility.test.ts`：`1`。

五个 direct files细分：

- `assembly-replica-dispatch.test.ts`：`1`；
- `assembly-runtime-endpoint.test.ts`：`12`；
- `host-ingress.test.ts`：`8`；
- `router-default-spawn-probe.test.ts`：`2`；
- `service-error-cross-layer-convergence.test.ts`：`2`。

没有运行完整 Router suite。

## 7. Scope与反向审计

production reader反向搜索中没有：

- ServiceDeployment v2；
- DeploymentArtifact v2；
- GatewayEntry v1；
- compatibility alias、fallback或dual read。

任务授权的三个 direct runtime-wire tests仍有明确命名的
`LEGACY_HTTP_WIRE_GATEWAY_ENTRY_IDENTITY` / `LEGACY_TEST_WIRE_GATEWAY_ENTRY_IDENTITY`：
它们属于当前 `runtimeAssemblyRequest` 的 HTTP/test wire branch，该 branch仍显式冻结为 v1，
不是 filesystem/current artifact reader。把该 wire升级到 v2需要修改任务写集外 protocol/
dispatcher consumer，本 leaf没有越界；current reader与本 leaf新增 fixture只接受/产生 v2。
其余旧 generation hit仅为 stale-version negative tests。

未派子 Agent；未启动 server、network、stable instance、watch或 live selector。验证所需
`node_modules` 只临时链接已安装 dependency tree，命令结束后已删除。未 merge、rebase或 push。
