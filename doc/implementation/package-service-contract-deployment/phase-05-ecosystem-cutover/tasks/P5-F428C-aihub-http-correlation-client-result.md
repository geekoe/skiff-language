# P5-F428C AIHub HTTP correlation-free client stream result

状态：`COMPLETED`。没有触发 `TASK_SCOPE_EXPANDED` 或 `TASK_NOT_EXECUTABLE`。

AIHub browser client 已删除旧 WebSocket 遗留的 HTTP correlation producer/consumer：

- browser 不再生成、记录或传递 request ID；
- `streamChatEvents` 直接发送 caller 提供的业务 body，不注入 correlation field；
- SSE parser 只要求 canonical `{type, seq, event}` envelope；
- fixture、完整 sequence 与 fail-closed tests 均使用新 envelope shape。

本 leaf 没有新增 correlation header、query、cookie、SSE `id:`、同义字段或 client correlation map。
单个 `fetch` Promise、response body reader 与既有 lexical `AbortController` 继续拥有当前 HTTP response。

## 1. 精确输入与提交

| 角色 | commit | tree |
| --- | --- | --- |
| Internals exact input | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` |
| Internals implementation | `ddea4f32d4b40b5db95ebac27fe9f4681bd2e4e3` | `46f77a3ff9d5eddac7ca8cfdc91b43269666e351` |
| Skiff production evidence anchor | `95efdf357a647d549bac047f5d301905df843dd3` | `2285dbdf3d0d3421f226528553f99f95b68769e7` |
| Skiff result checkout input | `1f52b2f5053830134e59bfa6f5c67d787078efa2` | `d859b21fbbbf8c1c3db724af53ebf3654e0c3a94` |

Internals implementation branch是 `codex/p5-f428c-aihub-client`，提交后 clean。本 result 在同名
Skiff task branch 单独提交；result commit/tree 在交付时从提交后 repository state 返回。

五分钟启动门禁已满足：第一次实际代码修改直接落在三个授权 client 文件，没有因未知量扩大范围。

## 2. Implementation 与 wire 证据

### 2.1 Browser owner

`aihub/client/app.js` 的 send path 现在只构造业务 body、记录该 body，并把
`{url, body, signal, callbacks}` 交给 transport。`createRequestId` generator 已整段删除。
`AbortController` 的创建、active controller ownership、Cancel handler、AbortError handling与
finally cleanup均未修改。

### 2.2 Transport 与 parser

`aihub/client/chat-stream.mjs`：

- `streamChatEvents` 参数列表不再包含 correlation transport参数；
- transport 使用 `JSON.stringify(body)`，没有 body clone/injection；
- headers仍精确只有既有 SSE accept与JSON content type，没有 correlation carrier；
- `parseEnvelope` 只验证 `type === "aihub.llm.event"`、non-negative safe-integer `seq`和带 string
  `tag`的 object `event`；
- parser不要求或读取 envelope request/run字段。

terminal state machine、`[DONE]`规则、finite SSE/error-body limits、fatal UTF-8 decoder、
reader cancel/release与 AbortError透传代码均未修改。

### 2.3 Tests 与 nested business identity

`aihub/client/chat-stream.test.mjs` 的 `eventEnvelope` fixture现在精确生成：

```json
{"type":"aihub.llm.event","seq":0,"event":{"tag":"request-start"}}
```

完整 sequence test 的 exact POST body只含 `provider`、`model`、`messages`和`stream`业务字段。
同一 sequence继续覆盖 reasoning/text item ID、`tool-1` start/delta/end聚合输入、base64与finish；
nested provider/tool lifecycle IDs没有删除或改名。既有 browser reducer本身没有 diff。

## 3. 自验收矩阵

| 任务条款 | 状态 | 代码 / 测试证据 |
| --- | --- | --- |
| 删除 browser generator、日志注入、transport参数与body注入 | PASS | `app.js` send path只记录/传递业务body；generator删除；transport直接 stringify body |
| 不新增 correlation carrier或map | PASS | request headers仍只有 `accept` / `content-type`；授权文件 correlation反搜为0 |
| parser只消费 canonical envelope | PASS | `parseEnvelope`只读取/验证 `type`、`seq`、`event`及 event tag |
| request body exact assertion只有业务字段 | PASS | 完整 sequence test 对 JSON body做 exact deep equality |
| fixture全部采用新shape | PASS | 唯一 `eventEnvelope(seq, event)` helper及 malformed fixture均无外层ID |
| reducer、terminal、buffer、cancel、abort语义不变 | PASS | 对应 production函数无diff；聚焦 suite 18/18 |
| 保留 nested provider/tool IDs | PASS | sequence保留 reasoning/text IDs与同一 `tool-1` start/delta/end |
| malformed envelope、terminal前EOF、terminal后event fail closed | PASS | 对应 protocol error tests全部通过 |
| invalid UTF-8与buffer limit fail closed | PASS | fatal decoder与finite buffer test通过 |
| reader error触发cancel，AbortError原样透传 | PASS | network-error cancel assertion与AbortController signal identity test通过 |
| 精确写集 | PASS | implementation diff只有三个授权 client文件 |

## 4. 验证账

在 exact implementation commit上执行：

| 命令 | 结果 |
| --- | --- |
| `node --test aihub/client/*.test.mjs` | PASS：18 tests，0 fail，0 skip |
| `node --check aihub/client/app.js` | PASS |
| `node --check aihub/client/chat-stream.mjs` | PASS |
| `node --check aihub/client/chat-stream.test.mjs` | PASS |
| `git diff --check` | PASS；提交前 working diff无whitespace error |
| `git diff --check ed5d333b2406d5375fca8acc96f4695667c48ced..HEAD` | PASS |

父审计第8节的 client反向搜索：

```bash
rg -n -i \
  '(request_id|requestId|runId|runIdFromRequestId|correlationId|correlation_id)' \
  aihub/client/app.js \
  aihub/client/chat-stream.mjs \
  aihub/client/chat-stream.test.mjs
```

结果：`0` match。

保护域检查：

```bash
git diff --exit-code ed5d333b2406d5375fca8acc96f4695667c48ced..HEAD -- \
  packages/llm-api packages/llm-providers aihub/service codex-relay
```

结果：PASS，`0` diff。implementation的完整 changed-file list精确为：

```text
aihub/client/app.js
aihub/client/chat-stream.mjs
aihub/client/chat-stream.test.mjs
```

整个 `aihub` 的 correlation zero-match属于 F428B service leaf合流后的 combined gate；本 client-only
leaf没有把尚未合入的 parallel service变更伪报为自身证据。

## 5. 禁令与后继边界

- 没有修改 AIHub service、README、static server、HTML/CSS、其它 Internals domain、
  Skiff production或skiff-packages。
- 没有运行 build/dev/start、stable watch、artifact reload、instance、fixed port、live、
  真实provider或browser E2E。
- 没有生成 receipt或执行 identity comparison；这些仍由两leaf合流后的唯一 combined owner负责。
- 没有 merge、rebase或push，也没有自行承接 combined节点。
- Internals implementation提交后状态clean；Skiff result提交后状态在交付前单独复核。
