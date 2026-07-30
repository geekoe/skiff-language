# P5-F440Z3C Current GatewayEntry wire identity preflight

状态：Ready。只读审计；确定 RuntimeAssembly request 各真实分支从 GatewayEntry v1
hard cut 到 current v2 的完整 owner，解除 Z3B。

## 直接父节点

- `P5-F440Z3B-router-websocket-rpc-gateway-integration-result.md`
- `P5-F440Z1-router-websocket-rpc-snapshot-result.md`

Z3B 已用真实 loopback 证明：current physical binding携带 v2 identity，而TypeScript
`websocketConnect` request wire仍要求v1，upgrade因此HTTP 500。Z1同时记录HTTP/test wire branch仍有
legacy v1。Rust current `GatewayEntryIdentity` parser只接受v2。

## 目标

只读追踪所有 RuntimeAssembly request producer、validator、schema、Rust transport consumer与
cross-system corpus，回答：

1. `http`、`websocketConnect`、`websocketJsonRpc`、test入口各自当前生产producer传入的是v1还是v2；
2. TypeScript lexical/metadata/schema分别接受什么；
3. Rust transport/request/eval/Host分别接受什么；
4. 哪些v1命中是current positive fixture，哪些是故意的stale negative；
5. 应一次hard cut所有gateway request branch，还是只需先cut connect；给出代码依赖依据；
6. 精确implementation写集、测试写集、cross-system fixture与必跑non-live命令；
7. implementation能否作为一个有界checkpoint完成；若需拆分，列最小顺序DAG和共享owner。

不得只搜索prefix计数；必须沿真实Gateway/runner producer到Rust consumer验证每个分支。

## 允许范围

只读：

- `router/src/protocol/`
- `router/src/gateway/`
- `router/src/router/`
- `router/tests/`
- `runtime/`
- `artifact-model/`、`artifact-identity/`
- `cross-system-fixtures/package-service-ecosystem/`
- 与这些owner直接相关的父任务/result

只新增本任务result：

`P5-F440Z3C-current-gateway-entry-wire-identity-preflight-result.md`

禁止修改production、test fixture、schema、README/checker或其它task/result。不得运行stable、
network、live或完整suite；通常无需运行测试。不得派子Agent。

## 必须给出的矩阵

Result至少包含：

| Request branch | TS producer identity | TS validator/schema | Rust consumer | Current一致性 | Exact owner |
| --- | --- | --- | --- | --- | --- |
| HTTP unary/stream | | | | | |
| WebSocket connect | | | | | |
| WebSocket JSON-RPC | | | | | |
| test/runtime assembly | | | | | |

并列出：

- 全部production v1 owner；
- 全部current positive v1 test/corpus路径；
- 保留的v1 stale-negative路径与理由；
- Z3B恢复前必须完成的最小checkpoint；
- 可留给F0的纯fixture/tooling跟随项；
- 若HTTP或test真实路径也已断，指出最早不会被上游遮挡的直接探针。

## 停止与交付

10分钟内形成单一明确范围；如果仍有多个会改变实现方向的未知量，返回
`TASK_NOT_EXECUTABLE`和精确缺口，不继续扩展审计。

- worktree：
  `/Users/geek/workspace/skiff-p5-f440z3c-gateway-wire-preflight`
- branch：`codex/p5-f440z3c-gateway-wire-preflight`

只提交result；不merge/rebase/push。
