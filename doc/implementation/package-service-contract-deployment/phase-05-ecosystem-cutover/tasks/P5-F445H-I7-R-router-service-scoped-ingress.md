# P5-F445H-I7-R Router service-scoped ingress consumer

状态：`COMPLETE`。

## 1. Parent and baseline

直接父节点：

- `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md`

权威语义来自：

- `doc/architecture/package-service-contract-deployment.md`
- `doc/architecture/runtime-deployment-topology.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- `doc/reference/runtime.md`

| 项 | 值 |
| --- | --- |
| baseline commit | `1a11328a241b5d177eb40885e294fe31d65a7240` |
| baseline tree | `ca1f7c2f040458df4275d00801eb0fc61046a1a8` |
| branch | `codex/p5-f445h-i7-r-router-ingress` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-r-router-ingress` |
| integration owner | `/root/phase05_integration_steward` |

## 2. Scope

本任务只迁移 Router consumer：

- Router 严格读取 `x-skiff-service` 与 `x-skiff-version`，先在 active assembly 中选择唯一精确
  deployment，再在该 deployment 内按 HTTP method/path 或 WebSocket path 选择入口；
- HTTP `Host` 只作为业务请求 metadata 透传，不参与 deployment 或 handler 选择；
- 不同 service 可以声明相同 selector，同一 service 内重复 selector 继续 fail closed；
- Router 产生的 runtime frame 使用 `skiff-runtime-frame-v2` 并携带精确 deployment；
- WebSocket upgrade 使用相同的 service/version 选择，并在整个连接生命周期固定精确 deployment 与
  assembly generation；
- 缺失、非法、未知或歧义 service/version，以及跨 deployment frame 替换全部 fail closed。

写集限于 Router TypeScript production、直接 Router tests/fixtures，以及 Router frame producer确需的
transport consumer测试。本任务不修改 compiler/authoring、deployment resolver/loader/linker、Rust Host
activation、canonical K owner，也不在 Skiff 内实现 external Host/local ingress 映射。

## 3. Evidence

必须建立真实 RED 后再实现 GREEN，并至少完成：

```text
pnpm --filter @skiff/router test
pnpm --filter @skiff/router type-check
```

同时运行受影响的 protocol、HTTP、WebSocket、snapshot 聚焦测试，反向搜索旧 Host selector、
`skiff-runtime-frame-v1` Router producer和缺少精确 deployment 的 request frame。

完成后写同名 `-result.md`，提交实现与结果，并把 exact commit/tree 交给 integration owner；不写
integration、不 push、不运行 stable/live/network/Mongo。
