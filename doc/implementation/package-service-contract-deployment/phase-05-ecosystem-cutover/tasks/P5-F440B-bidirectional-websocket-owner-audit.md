# P5-F440B 双向WebSocket JSON-RPC owner审计

状态：Ready。高风险、只读shared broker/runtime审计。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`

父节点已经覆盖F439B的outbound-only前提并冻结新wire。只读本任务和父节点引用的权威设计；不得从F439C
过时协议段恢复旧规则。

## 目的

一次列清从`websocket.yml.jsonRpc` typed projection、peer frame分类到Router broker、RuntimeDispatcher、
runtime typed handler与response写回的全部owner，并同时保留Skiff-originated
`requestJsonToConnection`挂起/恢复链。输出可并行的实现DAG，不实现。

## 只读范围

- `std/**`
- `compiler/**`中std callable、generic native、effect/suspension、external adapter projection
- `artifact-model/**`、`artifact-identity/**`、`deployment/**`
- `runtime/**`中linked codec、request/eval/native、transport、cancel、dispatcher adapter
- `router/**`中WebSocket gateway、connection/generation索引、RuntimeDispatcher与protocol
- `test-runner/**`、`scripts/**`、`cross-system-fixtures/package-service-ecosystem/**`
- 父节点及其直接权威引用

唯一允许写入是本leaf result。禁止修改production/test/design、Internals、stable/live。

## 必须回答

1. 画出当前`connection.send` definition→compiler→runtime producer→transport→Router consumer→socket链，
   锁定raw send保持non-suspending的测试。
2. 找出现有可复用的挂起host operation、gateway dispatch、typed codec、deadline/cancel和exactly-once
   completion原语；明确哪些只能借机制、不能共享pending owner。
3. 冻结shared authoring/artifact checkpoint：
   - `websocketJsonRpc` adapter kind；
   - `websocket.jsonRpcParams`、connectionId、businessIdentity合法阶段；
   - per-method selector/key/identity、params/result schema与unary限制；
   - old receive/transport id/batch/binary/notification fail-closed。
4. 冻结profile-neutral broker内部接口和两个独立状态表：
   - outbound runtime correlation→opaque profile/id→peer response；
   - inbound typed id/method→pinned-generation ingress→dispatcher correlation→peer response；
   - direction、connection/socket generation/profile/id key与各自tombstone。
5. 给出`jsonrpc-2.0-text`完整frame classifier：request/response/notification、string/safe-integer/null id、
   strict result-vs-error、parse/invalid/batch、unknown method、invalid params、binary和伪造response。
6. 核对所有race：同值双向id、active/tombstoned duplicate、乱序、cancel-before/after-complete、
   deadline、socket/runtime disconnect、generation replacement、late response/completion、容量与tombstone
   驱逐；每种必须指定唯一terminal owner。
7. 冻结RuntimeDispatcher inbound桥接和runtime adapter：
   - handler参数/返回typed codec；
   - void→null；
   - expected result union；
   - uncaught throw→sanitized internal；
   - peer cancel/disconnect→不可捕获terminal；
   - transport id永不进入业务值。
8. 列出TS/Rust schema/generation、direct fixture、negative legacy fixture、README/checker与聚焦测试。
9. 输出互斥实现DAG：shared schema/compiler、std/runtime transport、runtime execution、Router broker/profile、
   fixture/tooling、focused combined、独立验收。每个节点写首次修改、selector和证据失效边界。
10. 如果现有owner迫使broker核心解析业务JSON、两个方向必须共享同一pending map，或还存在会改变公开
    API/wire的未决项，返回`TASK_NOT_EXECUTABLE`并停止。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440b-bidirectional-websocket-audit`
- branch：`codex/p5-f440b-bidirectional-websocket-audit`
- result：`P5-F440B-bidirectional-websocket-owner-audit-result.md`

新增并提交唯一result；返回commit/tree、双向状态机、owner/DAG、验证矩阵和clean状态。不得派子agent。
