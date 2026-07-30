# P5-F420B Retire RuntimeAssembly WebSocket residue result

状态：`TASK_SCOPE_EXPANDED`。F420A 暴露的 RuntimeAssembly WebSocket 残留已在授权的
`router/**` 范围内完整删除，Router full suite、TypeScript、F420/F420A 的 Router 与
current-generation 门禁均已闭合；但完整 tooling gate 与 workspace format gate 暴露三个
候选零 diff、位于 `scripts/**` / `test-runner/**` 的范围外失败，因此 F421 **未解除**。

## 1. Exact candidate 与实现 checkpoint

- integrated start / tree：
  `697246a15b9e1942b2e05be19d32d3c039c81786` /
  `12734a8e1754b2898b55854df30c0e52559848da`；
- task checkout / tree：
  `c5d36d65e534cdd4f0504d2b837c8e95663eefc8` /
  `54194dde9db0a443556b05adafaff64e5b2c2fa4`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- F420B implementation / tree：
  `7a6b9af64435704063b104022dd86889fa1ecae0` /
  `70793edc2e2bee687e6828020442d8dcc441438e`。

启动时 integrated start、task checkout 与 accepted F415 均经
`git merge-base --is-ancestor` 验证为 HEAD ancestor。

## 2. 删除的 production 与 public surface

本节点删除：

- `router/src/gateway/assemblyWebSocketGateway.ts`；
- `router/src/index.ts` 中 `AssemblyWebSocketGateway` public export；
- `router/src/router/server.ts` 中旧 gateway 的构造、listen、startup output 与 shutdown
  接线；
- `router/src/router/assemblyRuntimeRegistry.ts` 中仅服务旧 WebSocket ingress 的
  `CanonicalAssemblyWebSocketIngressIdentity`、canonical args、hash helper及相关 import。

Router current production 只从 RuntimeAssembly v2 启动 HTTP ingress。下列通用 WebSocket
owner保持不变：

- `router/src/gateway/webSocketGateway.ts`；
- `router/src/gateway/webSocketConnectionLifecycle.ts`；
- `router/src/router/webSocketGenerationLifecycleRouter.ts`；
- `router/src/router/runtimeEndpoint.ts`；
- `router/src/router/runtimeDispatcher.ts`；
- `router/src/protocol/**`。

`connection.send` 的普通 WebSocket wire、sender trust、close race和observability继续由实际
测试覆盖，没有恢复 RuntimeAssembly WebSocket binding。

## 3. 删除与保留的测试

删除两个只证明旧 RuntimeAssembly WebSocket 正例的文件，共 15 个测试实例：

1. `assembly-websocket-gateway.test.ts`：14 个；
   - repeated metadata / registry-dispatcher-protocol peer；
   - 5 个非法 target 的 production-dispatch rejection；
   - 3 个 direct-send trust violation；
   - closed direct-send race；
   - failed upgrade safe fact；
   - receive close reason safe fact；
   - UTF-8 close reason truncation；
   - old-generation connection pin。
2. `assembly-websocket-ingress-identity.test.ts`：1 个；
   - old registry helper/digest identity。

混合测试只删除旧 Assembly WebSocket 半边。下列保留面实际列出并执行，共 62 个测试：

| 文件 | 数量 |
| --- | ---: |
| `websocket-gateway.test.ts` | 36 |
| `websocket-connection-lifecycle.test.ts` | 9 |
| `websocket-generation-lifecycle-router.test.ts` | 6 |
| `router-websocket-trust-dispatch.test.ts` | 3 |
| `runtime-endpoint-connection-send-trust.test.ts` | 5 |
| `service-error-cross-layer-convergence.test.ts` | 2 |
| `actor-production-routing.test.ts` | 1 |

`actor-production-routing.test.ts` 的固定 2026-07-25 deadline 已改为相对 deadline；没有改变
Actor production 语义。`loop-risk-health.test.ts` 只补 TypeScript 对 current request header
union 的明确 narrowing。

## 4. Router before/after 与 current positive inventory

F420A 基线：

```text
52 files
623 tests: 599 passed / 24 failed
tsc: 44 errors
```

F420B 候选：

```text
50 files
608 tests: 608 passed / 0 failed
tsc: PASS
```

总数下降精确等于删除的 15 个旧正例，没有通过 skip、selector 缩减或减少 verify plan
掩盖失败。

反向搜索结果：

- `AssemblyWebSocketGateway`：0；
- `canonicalAssemblyWebSocketIngressIdentity`：0；
- `assemblyWebSocketGateway` import/export：0；
- RuntimeAssembly WebSocket selector、routing、adapter旧字段只保留在
  `router-websocket-trust-dispatch.test.ts` 的明确 rejection fixture；
- `runtime-endpoint-connection-send-trust.test.ts` 中的 `websocketEntryId` 属于保留的通用
  `connection.send` wire/trust 测试，不是 RuntimeAssembly ingress 正例；
- current RuntimeAssembly positive snapshot只使用 HTTP binding或empty ingress。

没有恢复 shared DTO字段、protocol alias、dual reader或fallback。

## 5. 已执行门禁

| gate | 结果 |
| --- | --- |
| Router full listing | 608 listed |
| Router full execution | 608/608 PASS |
| Router TypeScript | PASS；0 errors |
| 保留面 focused listing/execution | 62 listed；62/62 PASS |
| Node 五文件组 | 36/36 PASS |
| identity single-source self-test | PASS |
| identity single-source checker | PASS |
| test-runner listing | 24 listed |
| test-runner execution | 23 passed / 1 ignored / 0 failed |
| `node scripts/run-skiff-tests.mjs` | PASS；2 canonical source entries |
| `node scripts/verify.mjs --only router` | PASS；608/608 |
| `node scripts/verify.mjs --only tooling` | FAIL；见下节 |
| `cargo fmt --all -- --check` | FAIL；见下节 |
| `git diff --check` | PASS |

所有任务列出的顶层命令均已执行；没有未跑的顶层 gate。`verify --only tooling` 在内部
`command-caller-migrations` 阶段失败后中止，其后续 plan 由 verifier 自身未继续执行，未伪报
为通过。

## 6. 范围外 blocker

### 6.1 Tooling command-caller migrations

精确命令：

```bash
node scripts/verify.mjs --only tooling
```

前两个 tooling 阶段分别通过 `7/7` 与 `1/1`；随后
`scripts/tests/command-caller-migrations.test.mjs` 为 `2 passed / 2 failed`。直接执行同一
test file得到相同首错：

```text
missing tar is reported through the safe outcome failure before remote I/O
expected: /failed to spawn tar: ENOENT/
actual:   error: skiff package publish requires --artifact-root

isolated status checked adapter rejects nonzero and invalid JSON before cleanup verification
expected: /node exited with 9/
actual:   isolated workspace ownership mismatch for <invalid> (nonce <invalid>):
          workspace ownership receipt is invalid
```

最小 owner 是 `scripts/tests/command-caller-migrations.test.mjs` 及其 package-publish /
isolated-workspace caller fixture：前者必须提供 current `--artifact-root` 前置条件，后者必须
提供 current ownership receipt，或由 tooling owner明确新的失败优先级。F420B write set
禁止修改 `scripts/**`。

### 6.2 Workspace format

精确命令：

```bash
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo fmt --all -- --check
```

首错位于：

```text
test-runner/tests/package_service_contract_deployment.rs:2003
```

rustfmt只要求把跨行
`project.package.artifact.package_local_abi.public_symbols["marker"]`
收敛为单行。F420B 未修改该文件且 write set 禁止修改 `test-runner/**`。

以下命令输出为空，证明三个 blocker 对应 owner相对 task checkout均无候选差异：

```bash
git diff c5d36d65e534cdd4f0504d2b837c8e95663eefc8 -- \
  scripts test-runner/tests/package_service_contract_deployment.rs
```

Router implementation已经闭合，不需要恢复共享 WebSocket artifact/protocol字段。要解除
F421，后继至少需要在授权范围内修复上述两个 tooling fixture前置条件与一个 rustfmt drift，
然后重跑完整 tooling/format gate。
