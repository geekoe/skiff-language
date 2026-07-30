# P5-F444C Agine service terminal connect-only cutover result

状态：`TASK_SCOPE_EXPANDED`。

本节点完成了 test-first RED，并把 service-only 终态草稿推进到 canonical package
expression validation；随后确认有两个独立阻塞不能在唯一允许写集 `agine/service/**` 内闭合：

1. `agine.ai/agent` 的 public package callable 参数仍携带 dependency-local canonical
   interface identity，Agine service 提供的是同 package id、同 symbol、同 ABI hash 的
   package-global identity，compiler 仍把二者判为不相等；
2. reference 已冻结 `timeout(15s) value { ... }`，但当前 exact Skiff production parser
   没有 timeout statement/value-expression lowering，且
   `std.websocket.requestJsonToConnection` 没有 per-call timeout 参数。

前者至少需要修改 `packages/agent/**` 或 Skiff package identity projection，后者需要修改
Skiff syntax/compiler/runtime。两者都越过本任务写入边界，所以没有伪造
`IMPLEMENTATION_PASS`，也没有提交不可通过的 implementation。

## 1. Test-first RED

production 修改前先扩展三个指定 Node receipt/architecture entrypoint，再运行：

```bash
node --test \
  agine/service/service-api-receipt.test.mjs \
  agine/service/internal/agine_service_architecture.test.mjs \
  agine/service/internal/host_runtime_architecture.test.mjs
```

真实 RED 为 `32 total / 24 passed / 8 failed`，同时命中：

- inline `service.yml` 仍拥有 36 条 HTTP entry，而不是 external `http.yml` 的 43 条；
- browser list/search 和五条 Host HTTP upcall 缺失；
- production `requestJsonToConnection` 调用为零；
- raw receive、DB relay、connection cache/current-directory `refreshRequested` 仍存活；
- `api.yml` 仍导出 legacy websocket surface。

这不是环境错误或预存 gate failure。

## 2. 已探明的 service-only 草稿

停止前的草稿只触碰 `agine/service/**`，包含以下方向：

- `service.yml` 收敛为唯一 `id`，新增 43-entry `http.yml` 和 connect-only
  `websocket.yml`，`api.yml` 收敛为空 mapping；
- 新增 private `host_peer_protocol`、`host_peer_rpc`、strict shared Host auth、browser
  Host-file HTTP handler和五条 Host HTTP upcall handler；
- connect 改用 non-generic `WebSocketConnectResult`，保留 browser/Host admission、
  max-one Host 和 exact active connection；
- list/search/current-directory 改为 exact connection 上的 direct peer request；
- 删除六个 `agine_ws_*`、`host_file_rpc`、`HostFileBrowseRequest`、
  `ChatStreamConnection`、raw envelope/cache/polling owner；
- 保留 server notification、`ToolProvider.activeConnectionId`、
  `HostToolAttempt` 和 durable tool/run/message identity；
- 把旧 dependency dot-call address 机械迁到 current slash-call address，使 canonical
  validation 能越过 call-target gate并暴露真正的 package identity 首错。

该草稿尚未完成 receipt 重写或 `.test.skiff` matrix，不是可合并 candidate。停止点聚焦
Node gate为 `32 total / 13 passed / 19 failed`；失败主要是旧 architecture assertions
仍读取已删除 raw WebSocket/DB relay owner，以及 entry 顺序断言尚未收敛。

为避免丢失调查成果，同时保证 implementation worktree clean，草稿保存在可恢复 stash：

```text
stash commit: 91f3cc32e9d6ce0b14b4145d3d94815ab1a52420
stash tree:   84c76648ce7069cb44b9aa72025bb8be30827266
scope:        79 files, all under agine/service/**
```

没有 implementation commit；branch 仍精确指向输入 commit。

## 3. Canonical identity blocker

终态 external manifests 生效后运行任务指定命令：

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  npm run type-check
```

cwd=`agine/service`。call address 使用 current canonical
`agent/tools.runtimeBindingsWithSubagent` 后，首错稳定为四个参数的 canonical identity
mismatch：

- `LlmClient` expected/found 都是 package `agine.ai/llm-api`、symbol `LlmClient`、ABI
  `65de703a...`；
- `AgentEventReceiver`、`ToolProvider`、`SubagentDelegate` expected/found 都是 package
  `agine.ai/agent`、ABI `02c07451...`；
- 唯一区别是 expected identity 内部仍表示为 `Dependency { dependency_ref: "agent" }`，
  found identity表示为 `PackageId { package_id: "agine.ai/agent" }`。

错误文本甚至同时报告 `expected any LlmClient, found any LlmClient`。使用旧 dot-call
address会先触发同一 identity error；迁到 slash-call address仍复现。尝试 public root path
则被正确拒绝为 `package dependency agent has no callable public path
runtimeBindingsWithSubagent`。因此 service 不能通过 cast、call spelling或 local record
construction把 dependency-local identity合法变成 package-global identity。

对干净 Internals 输入运行相同 workflow 会更早在旧 inline manifest停止：

```text
unknown field `http`, expected one of `id`, `kind`, `serviceCalls`
```

也就是说，该 identity 首错正是 terminal manifest migration让 canonical compiler首次继续
深入旧 service graph后暴露的 latent package/compiler blocker。

## 4. 15 秒 deadline blocker

当前 reference 明确写出：

```text
timeout(200ms) value { ... }
```

但 exact input的 `syntax/src/parser.rs` 没有 timeout/concurrent/value modifier parser或
对应 AST path；production `.skiff` 也没有可编译的 timeout block正例。
`std/websocket.skiff` 的 request signature仅为：

```text
requestJsonToConnection<TRequest,TResponse>(connectionId, method, value)
```

草稿中分别尝试 duration literal和整数 timeout spelling都在 parse阶段失败。仅依赖
`config.dev.yml` 的 120 秒 request deadline，或依赖 Host adapter自己的 15 秒 remote
deadline，都不满足本任务要求的“当前 HTTP execution 内 15 秒 caller deadline”。在 service
内手工抛 `TimeoutError`、轮询或增加业务 correlation同样违反冻结语义。

## 5. 未宣称的验收

由于任务正文要求范围扩张时立即停止，以下均未宣称通过：

- canonical `npm run type-check`；
- service `.test.skiff` success/auth/error/cancel matrix；
- final Node receipt/architecture GREEN；
- F444A §6 final reverse-search closure；
- implementation commit和可合并 candidate。

没有访问 stable artifact/watch/config，没有运行 live、browser或 network验证，没有修改
lockfile或 `node_modules`，没有 merge、rebase、push，也没有派 sub-agent。

## 6. 输入、停止点与状态

| Repo / worktree | HEAD | Tree | 状态 |
| --- | --- | --- | --- |
| Internals implementation | `19d41001f048efc0b70e13c21d105a855ddd86e2` | `15c48e07cc3d51794269719c606c87169bd0ee72` | clean；草稿在上述 stash |
| Skiff integration | `66ca01cc81f51c12304653e6d5eab0b2af1de4a4` | `89af3fa08f1605d3ce472ae8ef67226a5531e812` | clean |
| skiff-packages integration | `19cfab5dfc827450d37e1a103d21f31f8effa4f0` | `44081bd0498919086c13adea97c07722cb768352` | clean |

任务写出的 Skiff expected commit `534e75f1` 与实际 HEAD之间仅新增本任务文档；
production/reference文件零 diff。result worktree从同一实际 Skiff HEAD建立，本文是唯一新增文件。

后继必须先闭合：

1. package callable中 dependency-local / package-global interface identity的 canonical
   projection；
2. reference 已冻结的 timeout expression syntax/compiler/runtime implementation。

两项进入 integration 后，才能恢复 stash、完成 service receipt/test matrix，并重新建立
P5-F444C terminal candidate。
