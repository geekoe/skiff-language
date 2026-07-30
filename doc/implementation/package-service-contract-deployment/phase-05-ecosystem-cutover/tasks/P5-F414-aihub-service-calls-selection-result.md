# P5-F414 AIHub serviceCalls selection result

状态：Complete（静态迁移已合流；组合 isolated type-check 等待同批 Relay 前置）。

## 1. 提交锚点与范围

| 锚点 | commit | tree |
| --- | --- | --- |
| Internals start | `3a7234610c53b11c5f2cfdb5b04448408e924e31` | `72901006d82f6abafef3efb6567e4ccba6aa4caf` |
| agent implementation | `72c681647267d380a1364b3d117a2d12f90e3894` | `784ff83dc11b5b77a29becead5d2567c816bfad1` |
| integration cherry-pick | `ffb118a662c4c9ee7208d6f708d8645f40e4b03d` | `784ff83dc11b5b77a29becead5d2567c816bfad1` |
| final Skiff toolchain | `cfa5aadc7f864afbaa8109e3348690e9df3d1a42` | `319f8b0617c0d9c084a3d00dbf7e6c75271a9030` |

改动严格为：

```text
aihub/service/service.yml
aihub/service/service-api-receipt.mjs
aihub/service/service-api-receipt.test.mjs
```

`api.yml`、全部 `.skiff`、HTTP/WebSocket authoring 均未修改。

## 2. Selection 与 receipt

`service.yml` 只选择：

```yaml
serviceCalls:
  - managedLlm
  - providerCatalog
```

Package receipt 的 8 个公开 callable 仍完整可见；其中只有以下 5 个携带
`serviceOperationId`：

```text
managedLlm.streamChat
managedLlm.validateChat
managedLlm.webSearch
providerCatalog.builtinProvider
providerCatalog.model
```

`handleAihubHttp`、`selectProvider`、`websocket` 保持 package-only；interface declaration
也不能成为 executable operation。oracle 已切到 protocol v4 与 operation ID v1，并覆盖 selected
operation 缺失、package helper 被错误提升、interface method 被错误投影三类负例。

## 3. 验证与范围外前置

- `npm run test:service-api`：发现 7，`6 passed / 0 failed / 1 skipped`；
- skipped case 只在 canonical isolated workflow 提供生成 receipt 路径时执行；
- `git diff --check`：PASS。

独立 `SKIFF_ROOT=... npm run type-check` 在进入 AIHub receipt 前，被当时尚未合流的 Relay
legacy manifest 精确阻断：

```text
codex-relay/service/service.yml:
http.routes: invalid type: sequence, expected struct HttpGatewayEntryAuthoring
```

该 blocker 属于并行 P5-F413；P5-F414 没有越界修改 Relay 或共享 tooling。P5-F413 合流后由主 Agent
在同一 Internals integration 状态重跑组合 type-check。
