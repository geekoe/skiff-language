# P5-F428B AIHub HTTP correlation-free service stream result

状态：`IMPLEMENTED_WITH_KNOWN_D4_TEST_RUNNER_BLOCKER`。

AIHub service 的 HTTP preflight、canonical SSE envelope 和 post-start error helper 链已删除
旧 `request` / `run` correlation owner；没有触发 `TASK_SCOPE_EXPANDED`。leaf source、Node
unit/guard 与静态风险探针通过。assigned Skiff checkout 的已知 optional-handler test-runner
seam 在进入 AIHub source compile 前阻断 canonical type-check 与完整 service test，因此本文不把
被遮挡的 Skiff source tests 或 generated identity 记为 PASS。

## 1. 精确输入、写集与提交

| 输入 / 输出 | worktree / branch | commit | tree | 状态 |
| --- | --- | --- | --- | --- |
| Internals 输入 | `/Users/geek/workspace/internals-p5-f428b-aihub-service` / `codex/p5-f428b-aihub-service` | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` | clean |
| Internals implementation | 同上 | `a353e05d32beb04bc5ff19feba0308172a477ba7` | `3f3f94e18c8a7cd9150e88d32f32f8dbcc4d880a` | implementation-only commit，clean |
| Skiff result 输入 | `/Users/geek/workspace/skiff-p5-f428b-aihub-service` / `codex/p5-f428b-aihub-service` | `1f52b2f5053830134e59bfa6f5c67d787078efa2` | `d859b21fbbbf8c1c3db724af53ebf3654e0c3a94` | clean |
| Skiff result 输出 | 同上 | 见本 leaf 的独立 result commit / 交付回执 | 见交付回执 | 只新增本文 |

Internals commit 精确只修改任务允许的四个文件：

```text
aihub/service/internal/aihub_service.skiff
aihub/service/internal/aihub_service.test.skiff
aihub/service/internal/gemini.live.test.skiff
aihub/service/README.md
```

从 exact input 对以下禁止面执行 `git diff --exit-code` 均为零：

```text
packages/llm-api/**
packages/llm-providers/**
aihub/service/internal/managed_provider_transport.skiff
codex-relay/**
aihub/service/package.yml
aihub/service/api.yml
aihub/service/service.yml
aihub/service/service-api-receipt.mjs
```

没有修改 client、provider、其它 service、receipt owner、Skiff production 或
`skiff-packages`。

## 2. 实现结果

### 2.1 canonical correlation-free HTTP stream

`ChatEventsPreflight.ready` 现在只携带解析后的 `AihubLlmInput`。preflight 不再读取 request
body correlation 字段，也不再填充 transport fallback。`streamEnvelope` 的精确输出成为：

```json
{"type":"aihub.llm.event","seq":2,"event":{"tag":"text-delta","id":"text-1","text":"hello"}}
```

`streamEventSse`、chunk/error emitters、四层 post-start exception wrapper、
`streamChatEventResponse`、test adapter 与 HTTP handler 的整个参数链只传递 stream-local
state 和业务 event；旧派生 helper 已删除。

保留的语义：

- `seq:0` 的 `request-start`，随后按到达顺序递增；
- nested LLM event `id`、tool call identity 与 provider response/item identity；
- error 的 `providerCode` / `retryable` 业务诊断；
- consumer break/disconnect 沿同一 supervised stream ancestor 取消 provider work；
- 正常 `finish` 后才发送 `[DONE]`；post-start error 使用 next seq、end 且不发送
  `[DONE]`；
- pre-start failure 仍是有限 `application/json` response。

service-call/provider protocol、OpenAI-compatible unary completion surface、raw HTTP
server-stream handler signature、manifest 与 API 均未改变。

### 2.2 测试与 fixture

non-live source test 对 success 的九个 JSON SSE chunk 和三类 post-start error 的每个 JSON
chunk都解码为 map，并断言顶层恰好有三个 key：`type`、`seq`、`event`。这从结构上排除任意额外
correlation alias，而不是只抽查某两个旧名称。测试继续固定 request-start、provider item 顺序、
nested IDs、finish、usage、`[DONE]` 与 error next-seq 行为。

pre-start method/body/provider/validation/unavailable fixture 还断言有限 JSON 顶层恰好只有
`error`。普通 HTTP fixture 与 live Gemini fixture 的 request body 已移除旧 transport 字段；
live file 继续保留 `test defaultRun false`，本 leaf 没有运行它。

## 3. 验证结果

所有命令均在 assigned linked worktree 内运行，`SKIFF_ROOT` 固定为
`/Users/geek/workspace/skiff-p5-f428b-aihub-service`。没有访问 stable、live 或真实 provider。

| 验证 | 结果 | 计数 / 说明 |
| --- | --- | --- |
| `npm --prefix aihub/service run test:service-api` | PASS with expected skip | 8 discovered；7 pass、0 fail、1 canonical-receipt skip |
| `npm --prefix aihub/service run test:package-store` | PASS | 2/2 |
| `npm --prefix aihub/service run test:workflow-guards` | PASS | 13/13 |
| canonical correlation-free source probe | PASS | 四文件 forbidden search 为 0；精确 envelope/preflight/terminal/test-shape/defaultRun source assertions 全部通过 |
| `npm --prefix aihub/service run type-check` | BLOCKED before AIHub source compile | assigned Skiff `skiff-test-runner` 命中下述三个 D4 blocker |
| `npm --prefix aihub/service test` | BLOCKED after attributable Node tests | Node 共 23 discovered；22 pass、0 fail、1 expected skip；随后同一 D4 blocker |
| leaf 四文件 correlation reverse search | PASS | 0 match |
| 禁止面 exact-input diff | PASS | 0 diff |
| `git diff --check` | PASS | 0 error |

当前 branch 的全 `aihub/**` 搜索仍命中并行 F428C 独占的
`aihub/client/app.js`、`chat-stream.mjs`、`chat-stream.test.mjs`。F428B 没有越权修改这三个
文件；本 leaf 负责的 service production/test/docs 为零命中，也没有保留显式 negative-name
fixture。父节点的 whole-AIHub 零命中 gate 必须在 F428B 与 F428C 合流后判断。

### 3.1 已知且精确的 D4 blocker

assigned Skiff input 中 `DeploymentGatewayEntry.handler` 已是
`Option<PackageCallableId>`，但 test-runner 的两个 consumer 尚未适配：

```text
test-runner/src/canonical_test_gateway.rs:97
  expected Option<PackageCallableId>, found PackageCallableId

test-runner/src/package_test_assembly.rs:238
  cannot compare Option<PackageCallableId> with PackageCallableId

test-runner/src/package_test_assembly.rs:241
  Option<PackageCallableId> does not implement Display
```

两条 canonical 命令都在 Rust 编译 `skiff-test-runner` 时退出，尚未 publish Internals
package、编译 AIHub source 或执行 50 个 non-live Skiff test declaration。因此：

- source test coverage 已实现，但动态执行状态是 `BLOCKED`，不是 PASS；
- 本 leaf 不生成或比较 package/deployment/assembly receipt；
- `ServiceProtocolIdentity`、五个 operation ID、package schema/local ABI 与七个 gateway
  identity 的“不变”由未修改 authoring/signature/manifest/API 和禁止面 diff 支撑，generated
  exact comparison 仍由后继 combined owner负责；
- assigned Skiff production 不在本 leaf 写集内，没有为绕过 blocker 修改它。

## 4. 自验收矩阵

| 任务要求 | 结果 | 证据与限制 |
| --- | --- | --- |
| 删除 preflight、envelope、chunk/error 与 helper 链的 correlation | PASS | leaf 四文件大小写反搜为 0；production helper 链仅保留 seq/state/event |
| canonical SSE 精确为 `type + seq + event` | PASS | production source probe通过；success/error source test对每个 JSON chunk断言恰好三个允许 key |
| 保留 request-start、seq、nested/provider/tool IDs、诊断字段 | PASS | event projector和 provider/service-call范围零 diff；deterministic source fixture继续锁定顺序与 IDs |
| pre-start finite JSON；post-start next seq/end；仅正常 finish 后 DONE | IMPLEMENTED / EXECUTION BLOCKED | source probe和精确 source tests覆盖；canonical runtime execution被 D4 blocker遮挡 |
| cancel、item顺序、terminal与 raw HTTP stream不变 | SOURCE + NODE ORACLE PASS | cancellation ancestor Node oracle通过；handler/manifest/API不变；动态 Skiff test被遮挡 |
| non-live tests与 live fixture | SATISFIED IN SOURCE | success/error exact-shape assertions已落地；live source已更新、仍 `defaultRun false` 且运行次数为 0 |
| identity不变量 | SOURCE PASS / GENERATED COMPARISON DEFERRED | public authoring与协议依赖零 diff；combined owner在 F428B+C 后生成比较 |

## 5. 禁令与收尾

- 未运行 stable instance、watch、reload、router/runtime、固定端口、MongoDB 或真实 provider。
- 未运行任何 live test；`defaultRun false` fixture 仅更新 source。
- 未执行 merge、rebase 或 push。
- 未修改 Skiff production 或其它范围外文件。
- Internals implementation 与 Skiff result 分别使用独立 commit；不自行承接 combined 节点。
