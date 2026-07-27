# P5-F444A Agine service terminal owner preflight

状态：Ready。只读、有界的终局 owner 预检；不实现。

## 直接父节点

- `P5-F443B-cheap-combined-executable-resume-result.md`
- `P5-F439C-agine-host-jsonrpc-delta-audit-result.md`
- `P5-F440D-agine-host-peer-protocol-checkpoint-result.md`
- `P5-F440F-agine-host-private-jsonrpc-adapter-result.md`
- `P5-F440G-agine-client-host-http-cutover-result.md`

从这些父节点沿引用向上读取必要的当前权威设计。发生冲突时，以较新的 protocol checkpoint 和当前
`doc/reference/` 为准；不得把 F439C 中已被 F440D 覆盖的私有错误码表恢复为实现要求。

## 输入

| Repo | Root | Expected commit |
| --- | --- | --- |
| Skiff integration | `/Users/geek/workspace/skiff-phase-05-integration` | `eea50e12` |
| Internals integration | `/Users/geek/workspace/internals-phase-05-integration` | `2320949` |
| skiff-packages integration | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `19cfab5d` |

三棵输入必须 clean。Internals 与 skiff-packages 仅只读。

## 要回答的问题

1. 给出 Agine 最终三个 authoring 文件的精确形态：
   - `service.yml` 保留哪些 key；
   - `http.yml` 是否机械承接当前 36 条 entry；
   - `websocket.yml` 的 `path`、connect handler 和 adapterArgs；
   - Agine 只主动调用 Host 的三个 method 时，`jsonRpc` 应缺省还是声明。必须用当前
     `service-yml.md` / `api-yml.md` 的 inbound/outbound 归属证明，不能沿用旧 audit 猜测。
2. 审计当前 `agine/service/**`，列出 Host 文件 list/search/current-directory 从 HTTP handler 到
   `std.websocket.requestJsonToConnection` 的完整目标调用图，以及当前仍存活的旧
   raw receive、event DTO、DB relay、polling/cache owner。
3. 对照唯一 `agine/protocol/fixtures/host-peer-jsonrpc-v1.json` 和 F440D，冻结 Skiff 私有 params/result
   record、business result union、平台错误投影及 exact current connection id 的 owner；业务类型中不得
   出现 transport `id` / `requestId`。
4. 列出应删除、替换、保留的精确 production 文件/符号和对应测试/receipt owner。特别说明：
   - connect callback 是否可直接复用当前函数，还是必须从旧 ingress-event dispatcher 中提取；
   - 旧 `agine_ws_dispatch.receive` 是否在其它当前业务上仍有合法 owner；
   - current-directory 是否仍有 `refreshRequested` / polling；
   - `api.yml` / `api/agine.skiff` 是否因 external HTTP 或私有 peer record发生变化。
5. 给出最小可执行实现 DAG：
   - 若单一 leaf 可在一个清晰写集内完成，给出一个实现任务的精确写集、RED、聚焦验证和反向搜索；
   - 若范围确实需要拆分，只能按互斥写集和显式依赖拆分，不能把同一 service 文件交给并行 leaf。
6. 明确重跑 F443B Gate C 所需命令，以及实现完成后是否还存在其它 production blocker。

## 允许读取

- 上述直接父节点及其必要向上引用；
- Skiff 当前 `doc/reference/{service-yml,api-yml,std-surface,runtime}.md`、`std/websocket.skiff`；
- Internals 当前 `agine/service/**`、`agine/protocol/**`；
- 为确认已完成 producer/consumer 边界，可读取 `agine/host/**`、`agine/client/**` 的直接关联符号和测试。

不要开放式审计其它仓库或恢复已删除的 live test 工作。

## 输出

只新增并提交：

`P5-F444A-agine-service-terminal-owner-preflight-result.md`

结论只能是：

- `PREFLIGHT_COMPLETE / TASK_EXECUTABLE`，带精确 DAG、写集和验证矩阵；
- `TASK_SCOPE_EXPANDED`，精确列出超出父节点的 production owner；
- `DESIGN_DECISION_REQUIRED`，只用于当前文档仍不能唯一决定实现的语义。

不得修改 production、test、fixture 或权威设计；不得运行 build、完整 type-check、stable、live、network；
不得 merge、rebase、push。最多运行只读搜索、语法检查或 test listing，不派子 Agent。

有界时间：15 分钟。到时若仍不能形成唯一实现路径，必须按工作流停止并如实上报。

worktree：

`/Users/geek/workspace/skiff-p5-f444a-agine-terminal-preflight`

branch：

`codex/p5-f444a-agine-terminal-preflight`
