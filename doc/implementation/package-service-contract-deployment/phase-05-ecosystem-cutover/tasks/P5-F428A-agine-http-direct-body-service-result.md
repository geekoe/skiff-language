# P5-F428A Agine HTTP direct-body service checkpoint result

状态：`IMPLEMENTED_WITH_CANONICAL_TEST_RUNNER_BLOCKED`。

Agine service/protocol 的 36-route HTTP wire checkpoint 已实现；其中 package owner 的
`/session`、`/track` 保持既有响应 contract，另外 34 条 Agine-owned route 已改为 direct
business body。HTTP correlation 字段、legacy response envelope 和 HTTP-to-WS DTO 复用均已
从对应边界移除，legacy WebSocket matching 保持不变。没有触发 `TASK_SCOPE_EXPANDED`。

两条规定 canonical 命令都在进入 Agine package 编译或 source test 前，被 assigned Skiff
worktree 中既有的 `skiff-test-runner` 类型不一致阻断。本结果因此不伪报完整 type-check 或
Skiff source test 已执行。

## 1. 精确输入与提交

| 输入 / 输出 | worktree / branch | commit | tree | 状态 |
| --- | --- | --- | --- | --- |
| Internals 输入 | `/Users/geek/workspace/internals-p5-f428a-agine-http-direct` / `codex/p5-f428a-agine-http-direct` | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` | clean |
| Internals implementation | 同上 | `f418038cc785f0e7929537cc3bcda321a5cdab24` | `62b88d5dabae6adb3d8f03a850908674f0a4f0f5` | implementation-only commit，clean |
| task 指定 Skiff production 证据锚点 | `/Users/geek/workspace/skiff-p5-f428a-agine-http-direct` | `95efdf357a647d549bac047f5d301905df843dd3` | `2285dbdf3d0d3421f226528553f99f95b68769e7` | task evidence |
| assigned Skiff result 输入 | 同上 / `codex/p5-f428a-agine-http-direct` | `1f52b2f5053830134e59bfa6f5c67d787078efa2` | `d859b21fbbbf8c1c3db724af53ebf3654e0c3a94` | 含后续 task-only dispatch/current-WS commits，clean |
| Skiff result 输出 | 同上 | 见本 leaf 的独立 result commit / 交付回执 | 见交付回执 | 只新增本文 |

第一次实际代码修改在启动后 5 分钟内完成。

Internals implementation 修改 25 个 task 允许文件：HTTP protocol/API、transport/dispatcher、
五个 HTTP domain adapter、为 neutral command 必需的四个 shared/WS owner、直接测试、
receipt/architecture checker、README 与受 owner 重命名影响的 lifecycle checker。没有修改
`agine/client/**`、`agine/host/**`、其它 service、Skiff production 或 `skiff-packages`。

## 2. 实现结果

### 2.1 精确 request surface 与 fail-closed guard

- `agine/protocol/http.ts` 删除 `RequestIdPayload`；22 个普通 HTTP TypeScript payload 只保留
  父审计矩阵中的业务字段，零字段请求使用 `Record<string, never>`。
- `api/agine.skiff` 的 34 个 Agine-owned HTTP payload 都只包含对应 route 的业务字段；
  28 个 schema `requestId` hit 已删除。
- `/chat/list`、`/hosts/activation-token`、OAuth start/disconnect 等零字段 route 也有独立 typed
  payload，并由 adapter 显式 decode `{}`。
- 删除 dead `ThreadToolProvidersAddPayload` 与 `ThreadToolProvidersRemovePayload`，没有新增
  HTTP route。
- dispatcher 在 typed decoder 和 session authorization 前检查
  `requestId`、`request_id`、`correlationId`、`correlation_id`。四项只要作为 JSON object
  key 出现就返回 `400 invalid_input`，不依赖 decoder 的 unknown-field 行为，也不回显值。
- `requestIdFromBody` 已删除。`chatId`、`messageId`、`runId`、`toolCallId`、`attemptId`、
  `clientInstanceId`、OAuth session 和 provider response identity 均保留。

### 2.2 direct HTTP response

34 条 Agine-owned route 统一使用：

```text
2xx: 直接 business object；没有业务字段时为 {}
non-2xx: {"error":{"code":"<code>","message":"<message>"}}
405: 同一 direct error body，并保留 Allow: POST
```

HTTP adapter 和 HTTP helper 不再产生 `eventName`、`requestId`、`ok`、transport outer
`payload`、`*-response` 或 `tool_call/receipt`。`/chat/llm-call` 的唯一 `payload` 是父审计
指定的业务字段，仍直接返回 `{"payload": ...}`。

`service-api-receipt.mjs` 新增顺序固定的 36-route wire matrix：两条 package-owned route
明确标为 unchanged，34 条 Agine-owned route 逐项记录 payload type、精确 request fields
和精确 success fields；`/chat/regenerate` 明确没有当前成功形态。统一 error shape 由
`httpError`、405 helper、`ApiError { code, message }` 与 direct-body tests 共同固定。

### 2.3 HTTP/WS transport 分离

新增以下 transport-neutral business command：

- `ChatCreateCommand`
- `ChatUpdateCommand`
- `ChatUpdateModelCommand`
- `ChatPinCommand`
- `ChatDeleteCommand`
- `ToolCallResultCommand`

HTTP adapter 直接从 HTTP-only payload 构造 command；legacy WebSocket 继续 decode 原
`*Input(eventName, requestId, ...)`，再适配成同一 command。两边最终调用
`thread_store.*FromCommand` 或 `tool_result_adapter.onClientToolResult` 的同一 business
owner。

旧 `successEnvelope`、`errorEnvelope`、`sendResponse`、`sendError`、WS DTO 和
WebSocket `requestId` matching 均保留在 WS 边界。为避免扩大到大量既有内部 source test，
`thread_store` 的 legacy `*Input` entry 保留为只做 command 转换的薄 wrapper；HTTP 与 WS
production 已不通过这些 wrapper 共享 transport DTO。

## 3. 验证结果

所有验证均在 assigned linked worktree 内执行；`SKIFF_ROOT` 固定为
`/Users/geek/workspace/skiff-p5-f428a-agine-http-direct`。没有访问 stable instance、live、
真实 provider 或浏览器 E2E。

| 验证 | 结果 | 计数 / 说明 |
| --- | --- | --- |
| `node --test service-api-receipt.test.mjs internal/agine_service_architecture.test.mjs` | PASS | 17/17 |
| `npm run test:workflow-guards` | PASS | 40/40 |
| `npm run test:architecture` | PASS | lifecycle/list 与 agent bridge 两项 checker |
| assigned `skiff-syntax` parser，遍历 `agine/service/**/*.skiff` | PASS | 所有 production/test `.skiff` source 可解析；这是 parser probe，不声称 source tests 已执行 |
| `SKIFF_ROOT=... npm run type-check --workspace @agine/service` | BLOCKED before Agine | `skiff-test-runner` 编译失败，未进入 package graph |
| `SKIFF_ROOT=... npm test --workspace @agine/service` | BLOCKED after attributable Node gates | workflow 40/40 与 architecture 先通过；isolated service receipt 随后命中同一 runner blocker |
| scoped HTTP correlation reverse search | PASS | `http.ts` 与五个 HTTP adapter 中 0 match |
| scoped HTTP envelope helper/label reverse search | PASS | 五个 HTTP adapter 中 0 match |
| WS/Host positive `requestId` search | PASS | WS service 108 match；unchanged Host 28 match |
| `git diff --check` | PASS | 0 error |
| scope/clean audit | PASS | Internals implementation commit 后 clean；Skiff 写入前 clean |

### 3.1 canonical blocker

两条规定命令均从 assigned Skiff worktree 构建工具。失败为：

```text
test-runner/src/canonical_test_gateway.rs:97
  expected Option<PackageCallableId>, found PackageCallableId

test-runner/src/package_test_assembly.rs:238
  expected Option<PackageCallableId>, found PackageCallableId

test-runner/src/package_test_assembly.rs:241
  Option<PackageCallableId> does not implement Display
```

三个 diagnostic 都位于 task 禁止修改的 Skiff production，且在 Agine package publish、
type-check 和 source test discovery/execution之前发生。没有为制造绿色证据越权修改 Skiff。

额外的临时、repo 外 compiler bootstrap probe 能发布 `std`、`llm-api`、`llm-providers`、
`agent` 与 `codex-relay`，随后又分别遇到 assigned input 已有的 AIHub expression-type model
failure，以及 `skiff.run/http-session` 的 database-state-requirement failure；因此也无法组成
完整 Agine dependency graph。临时 probe 只用于归因，没有写入或提交任何 repository 文件。

## 4. 自验收矩阵

| leaf 要求 | 结果 | 证据与限制 |
| --- | --- | --- |
| 1. 34 条 Agine route 只接受业务字段；两条 package route 保持 contract；删除 28 schema hit/raw echo | PASS | 36-route receipt + 34-route exact API field parser；HTTP scoped reverse search为零；`requestIdFromBody` 不存在 |
| 2. 四个 correlation alias fail closed | PASS IN SOURCE | dispatcher test 覆盖四项、400 direct error 与不回显；source test 已 parse，canonical execution 被上游 runner blocker 遮挡 |
| 3. 34 route direct success/error，405 保留 Allow | PASS IN SOURCE | common helper 静态 gate、36-route success matrix、success/business-error/405 direct-body tests；Node gates通过，Skiff source tests已 parse |
| 4. HTTP 不构造 legacy `*Input`，共享 neutral owner | PASS | HTTP adapter `*Input` 静态零命中；HTTP/WS owner-pair checker通过 |
| 5. legacy WS matching 保持 | PASS IN SOURCE | WS DTO/helper/request ID 正向 gate；delete WS envelope test保留 matching assertions |
| 6. 保留真实业务 identity | PASS | exact request matrix/API field oracle覆盖 task 列举字段 |
| 7. 删除 dead HTTP-looking payload，不新增 route | PASS | dead type absence oracle；route manifest/resolver仍精确 36 |
| 8. receipt/checker/tests 精确覆盖 request/success/error | PASS IN SOURCE | 36-route wire matrix、34 typed decoder counts、direct success/error/405/four-alias tests；完整 source execution受共同 blocker遮挡 |

最早风险探针已包含一个 direct success、一个既有业务 error、一个 405 和四个 forbidden
aliases；error test 同时断言 outer object 只有 `error`，inner object 只有 `code`、`message`。

## 5. 禁令与收尾

- 未迁移或修改 browser caller，也未合入 F426C WIP。
- 未启动、重载或修改 stable instance、router、runtime、watch、MongoDB 或固定端口服务。
- 未运行 live、真实 provider、browser E2E 或后继 combined probe。
- 未执行 merge、rebase 或 push。
- Internals implementation 以独立 commit 提交；Skiff 只以独立 result commit 提交本文。
- 本节点完成后未自行承接 browser 后继任务。
