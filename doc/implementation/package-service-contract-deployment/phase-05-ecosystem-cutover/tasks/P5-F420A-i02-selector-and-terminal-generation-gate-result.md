# P5-F420A I02 selector and terminal-generation gate result

状态：`TASK_SCOPE_EXPANDED`。I02 selector 与 Router current protocol identity 的机械遗漏已经
形成 implementation checkpoint；完整 Router gate 暴露需要独立架构决策的
RuntimeAssembly WebSocket 残留，因此 F421 **未解除**。

## 1. Exact candidate 与 checkpoint

- integrated start：
  `56501394220cf0751b599990761323402bbd0582`；
- task start / tree：
  `3475bdc1abf150a014834b8df5edb211e64e0e2b` /
  `9309930d3625b966075c64fbf17fa2b0c6bef505`；
- F420 implementation checkpoint：
  `5b4391eba8f19919b93a80ccdb637eb47a2585dc`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- F420A implementation checkpoint / tree：
  `33eee1d722f4da538fea0fe7b7a09d8e7d4fe7a1` /
  `2295e5e499574c67c1eac8fda7105de7d37d6baf`。

启动时三个要求的 commit 均经 `git merge-base --is-ancestor` 验证为 HEAD ancestor。

## 2. 修改前首错与 I02 修复

F420 记录的 Node 五文件组首错已精确复现：

```text
tests 36
pass  35
fail  1

actual URL: http://127.0.0.1:46000undefined
expected: /\/probe$/
```

`requestTypedUnary` 现在明确要求 `entrypoint.selector.protocol === "http"`，验证
`selector.host/method/path` 都是非空 current 字段且 path 为 absolute path，并且只用
`selector.method`、`selector.path`、`selector.host` 发出请求。没有读取
`unary.method/path/host`，没有 fallback、adapter 或 dual shape。

修改后同一 Node 五文件组：

```text
36 passed / 0 failed
```

现有 real-owner test 继续实际证明 `/probe`、两次零 artifact-I/O request、withdrawal 与
rollback。

## 3. Router current identity checkpoint

完整 Router verify 首轮还暴露 current manifest producer/fixtures 仍生成
`skiff-service-protocol-v3`，随后交给只接受 v5 的 runtime wire，造成 105 个级联失败。这是
F420 规定的 terminal generation 机械遗漏，已在授权的 `router/**` 内统一：

- `router/src/artifacts/identity.ts` 与 `router/src/manifest/loadManifest.ts` 只生成、校验 v5；
- current manifest fixture、helper、positive tests 与由 protocol identity 导出的 WebSocket
  gateway identity golden 更新为 v5；
- `router/tests/protocol.test.ts` 中 v3/v4 legacy rejection 负例保持不变。

机械收敛后 F420 Router 五文件组重新执行：

```text
5 files passed
164 tests passed / 0 failed
```

## 4. 已完成验证

| gate | 结果 |
| --- | --- |
| Node 五文件组 | 36/36 PASS |
| identity single-source self-test | PASS |
| identity single-source checker | PASS |
| Router focused listing | 164 listed |
| Router focused execution | 164/164 PASS |
| test-runner listing | 24 listed |
| test-runner execution | 23 passed / 1 ignored / 0 failed |
| `node scripts/run-skiff-tests.mjs` | PASS；2 canonical source entries |
| `git diff --check` | PASS |

`run-skiff-tests` 首次启动被本机仅约 348 MiB 可用空间阻断，MongoDB 因低于 512 MiB
安全阈值拒绝建索引；清理可重建的 shared Cargo incremental cache 后可用空间约 9.2 GiB，
原命令重跑通过。这不是 product failure。

## 5. Dynamic fixture 与 current/legacy inventory

沿用 F420 official generator 的真实记录：

- producer commit/tree：
  `4c719b33131fff39a2f8f2e692b88b4710aae892` /
  `2cdde01a073205c004329fc2c4fbe93943a9b98b`；
- case：
  `cross-system-fixtures/dynamic-build-id-parity/case.json`；
- service unit：
  `units/services/example~com~~dynamic-golden/2026.06.04.json`；
- dynamic build：
  `skiff-service-build-v1:sha256:ed32b93ba8d48f7cb93cb4ef13720e943eec758b3e87a757eaec32b0a290ed26`；
- service unit identity：
  `skiff-service-unit-v1:sha256:1ed23e89365f01cde88881a41d5f13a14895fc899d39be255ef9f3a9e98c81c7`。

current positive 保持 PackageArtifact v9、Local ABI v7、package build v10、PackageUnit v2、
ServiceContract / service protocol v5 与 RuntimeAssembly v2。Router production/current fixtures
已无 v3/v4 protocol identity；v3/v4 只保留在 runtime protocol legacy rejection tests。
PackageArtifact v8、canonical build v9、RuntimeAssembly v1 仍只作为明确失败关闭负例。

concrete callable `may_suspend`、service selection、exact operation identity 及 F415
`collection_name_mapping` 均由通过的 test-runner gate 保留；没有修改其 production owner。

## 6. 范围扩张 blocker

Router full verify 收敛为：

```text
52 test files: 47 passed / 5 failed
623 tests:     599 passed / 24 failed
```

其中 23 个失败都来自同一矛盾：

```text
RuntimeAssembly ingress currently accepts only HTTP
```

当前 `RuntimeAssemblyIngressBinding` 与 request header 已删除 WebSocket 所需的
`contract`、`contractOperationId`、`websocketAdapter`、`websocketEntryId` 和
`gatewayEntryIdentity`，但以下 production consumer 及其 tests 仍把 WebSocket 当作
RuntimeAssembly ingress：

```text
router/src/gateway/assemblyWebSocketGateway.ts
router/src/router/assemblyRuntimeRegistry.ts
```

`tsc --noEmit` 因同一矛盾产生 44 个错误。闭合它必须在两个方向中选择一个：

1. 把 WebSocket contract/selector/header 重新纳入 RuntimeAssembly；或
2. 删除/替换遗留 assembly WebSocket gateway，让现有非-assembly WebSocket 路径成为唯一
   owner。

这会改变 WebSocket public/runtime 语义，不是 current generation 的机械字段替换，F420A
不得自行选择。

另有一个独立 test-only failure：
`actor-production-routing.test.ts` 的固定 deadline
`2026-07-25T01:00:00.000Z` 已过期，运行时报告
`unknown Actor invocation invoke-1` 后测试超时。该项可由后继改为确定性未来/相对 deadline，
不涉及 production。

由于已经出现语义性 production owner，本节点按任务要求停止；没有运行
`verify --only tooling`、`cargo fmt --all -- --check`，也没有为通过 gate 而恢复旧 WebSocket
字段或删除 production。F421 继续被阻断。
