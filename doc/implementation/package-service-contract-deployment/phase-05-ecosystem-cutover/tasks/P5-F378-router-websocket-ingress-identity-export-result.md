# P5-F378 Router WebSocket ingress identity export result

状态：**TASK_SCOPE_EXPANDED / SAFE CHECKPOINT**。Router production 的 ESM
missing-export 断链已经闭合，canonical helper 恢复为单一 owner，直接 import、Router
startup-link probe、identity 摘要冻结测试、相邻 assembly registry 与 HTTP gateway 回归均通过。
但当前基线还把共享 RuntimeAssembly ingress/request 类型与校验收窄成 HTTP-only；Router typecheck
和既有 WebSocket assembly suite 因而在进入本 helper 前失败。恢复这些字段和路由校验会修改本任务明确
禁止的 assembly DTO、selector/envelope 与 WebSocket 业务路由协议，本节点没有越界处理。

## 1. Exact checkpoint 与边界

| 项目 | commit | tree |
| --- | --- | --- |
| 任务基线 | `087ada637bd845a603734826fa4eb80c48138a56` | `330e09e244830b4bb8ed7bd2543b1fb013d08749` |
| production/test safe checkpoint | `fa8dd6d9682b2d9ac5981645e0120af44062bc48` | `8c70430b0a843ed488d05fe525df879be05beea6` |

- worktree：
  `/Users/geek/workspace/skiff-p5-f378-router-websocket-identity-export`；
- branch：`codex/p5-f378-router-websocket-identity-export`；
- production 只修改
  `router/src/router/assemblyRuntimeRegistry.ts`；
- 新增直接 identity/export 测试
  `router/tests/assembly-websocket-ingress-identity.test.ts`；
- 没有修改 RuntimeAssembly/artifact DTO、request envelope、selector、WebSocket 路由语义、
  HTTP gateway、Host、runtime、test-runner、其它仓库、stable 或 live 状态。

## 2. 已闭合的 canonical owner/export

Git 历史 `d11a3dc2^`、现有 production import 和既有 frozen digest 测试共同确定：

- `assemblyRuntimeRegistry.ts` 是
  `canonicalAssemblyWebSocketIngressIdentity` 的原 canonical owner；
- `assemblyWebSocketGateway.ts` 只通过
  `canonicalWebSocketIngressIdentity = canonicalAssemblyWebSocketIngressIdentity`
  re-export，同一函数引用，不拥有第二套实现；
- helper 复用原有 `stableStringify`、`sha256Hex`、canonical host 与既定 body 字段：
  `adapterArgs`、`contractOperationId`、canonical selector、`serviceId`、
  `serviceProtocolIdentity`；
- `websocketEntryId` 与 `gatewayEntryIdentity` 继续共享同一 digest，只保留各自既定 prefix。

新增测试同时断言 gateway export 与 registry owner 是同一函数对象，并冻结已有摘要
`c85b1bb0...0414`。反向搜索结果：

```text
router/src 中 canonicalAssemblyWebSocketIngressIdentity function 定义：1
gateway production import：1
gateway compatibility re-export：1（同一函数引用）
悬空同名 import：0
```

## 3. 通过的验证

### Direct import 与 startup-link

```bash
pnpm --dir router exec tsx --eval "<import registry and gateway; assert same function>"
```

结果：PASS，输出
`canonical websocket ingress identity import/export closed`。

随后从 `router/src/router/server.ts` 真实执行顶层 module linkage，并显式传入不存在的 task-owned
config path：

```bash
pnpm --dir router exec tsx --eval \
  "<import server.ts with --config /tmp/p5-f378-no-router-config.yml; \
    require failure to be config read rather than missing export>"
```

结果：PASS。模块成功穿过 `server -> assemblyWebSocketGateway ->
assemblyRuntimeRegistry` ESM 链，之后才按预期停止于 isolated config 不存在；不再出现
`does not provide an export named canonicalAssemblyWebSocketIngressIdentity`。

### 聚焦 identity、相邻 registry 与 HTTP 回归

```bash
pnpm --dir router exec vitest run \
  tests/assembly-websocket-ingress-identity.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/assembly-http-gateway-stream.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts
```

结果：**4 files / 23 tests PASS**。其中 identity/export 测试 1 个非零通过；
相邻 assembly runtime registry 和 HTTP gateway/HTTP unary suites 均非零通过。

`git diff --check`：PASS。

## 4. 范围外 blocker

### Router typecheck

```bash
pnpm --filter @skiff/router type-check
```

结果：FAIL，`47` 个 TypeScript error。直接原因不是新的 identity 算法，而是当前基线的共享
production 类型互相不一致：

- `router/src/router/runtimeAssemblySnapshot.ts:15-45` 只声明
  `protocol: 'http'`、非 nullable method，并从 binding 移除了
  `contract`、`contractOperationId`；
- 同文件 `runtimeAssemblyIngressKey` 在 `:200-205` 明确拒绝所有 WebSocket selector；
- `router/src/protocol/runtimeAssemblyRequest.ts:23-53,185-209` 只允许 HTTP routing，
  request header 也不再包含 WebSocket entry/adapter 字段；
- 未改动的 `assemblyWebSocketGateway.ts`、`assemblyControlPlane.ts` 及既有 WebSocket tests
  仍消费 canonical WebSocket shape，因此共同产生 type errors。

恢复 helper 后，该 helper 自身也如实暴露 shared binding 缺少 WebSocket
`contractOperationId`/`contract` 的同一 mismatch；用 cast、第二套局部 DTO 或宽泛 `unknown`
掩盖它会违背 canonical owner 和 fail-closed 边界。

### 既有 WebSocket assembly suites

```bash
pnpm --dir router exec vitest run \
  tests/assembly-websocket-gateway.test.ts \
  tests/router-websocket-trust-dispatch.test.ts
```

结果：FAIL，**2 files / 17 tests failed**。所有用例均在 fixture 构造
`RuntimeAssemblyIngressIndex` 时先被
`RuntimeAssembly ingress currently accepts only HTTP`
拒绝，尚未进入本次恢复的 identity helper 或 gateway dispatch。

历史 commit `d11a3dc2` 不只删掉 helper，也删掉 registry 的 WebSocket dispatch validation；
之后的 HTTP request/snapshot v2 commits `7c2161c7` 与 `d37418cc` 又把共享
ingress/request surface 收窄为 HTTP-only。当前 integration 因而同时保留早期
`assemblyWebSocketGateway` consumer 和后期 HTTP-only shared protocol。

要让上述 typecheck 和 WebSocket suites 通过，新的 shared protocol owner 必须先依据当前权威设计
决定是恢复 canonical HTTP/WebSocket union、nullable method、binding contract/operation fields、
request adapter/identity fields及对应 registry validation，还是从 production startup 移除尚未轮到的
WebSocket consumer。两条路径都会改变本任务禁止触碰的公共协议或业务路由表面，不能伪装成 export 小修。

## 5. 自验收矩阵

| 任务条款 | 代码证据 | 反向搜索 | 测试 |
| --- | --- | --- | --- |
| production import/export 闭合 | `assemblyRuntimeRegistry.ts` canonical function；gateway 同引用 re-export | function 定义 1；悬空 import 0 | direct import PASS；startup-link PASS |
| 复用既定 canonical 算法 | 原 `stableStringify + sha256Hex` 与冻结 body/prefix | 无第二个 helper/hash body | identity digest 1/1 PASS |
| Router typecheck | shared binding/request 当前 HTTP-only | 未改动 consumer 大面积 WebSocket shape mismatch | **FAIL，47 errors；范围外** |
| WebSocket assembly gateway | identity owner可直接加载 | production helper/export 已闭合 | direct identity 1/1 PASS；既有 gateway/trust 0/17，HTTP-only owner 阻塞 |
| 相邻 runtime assembly registry | `AssemblyRuntimeRegistry` 非 WebSocket dispatch 路径不变 | 无额外 registry owner | PASS，纳入 23-test batch |
| HTTP gateway 无回归 | HTTP production 未修改 | helper 只处理 WebSocket identity | HTTP stream/unary PASS，纳入 23-test batch |

Safe checkpoint 可以解除原 F375 的 ESM missing-export blocker；完整 Router/WebSocket
acceptance 仍需先合流新的 shared protocol owner，再在该精确候选重跑 typecheck 与 17 个既有
WebSocket tests。本节点没有 merge、rebase、push，也没有操作 stable/live。
