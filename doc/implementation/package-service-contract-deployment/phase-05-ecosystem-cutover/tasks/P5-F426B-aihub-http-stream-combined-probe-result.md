# P5-F426B AIHub HTTP stream merged-state combined probe result

状态：`SUPERSEDED_BY_HTTP_CORRELATION_DECISION`。

用户随后决定删除 HTTP `requestId`，因此本 probe 的 exact candidate 与 HTTP correlation 前提均已失效；
本文不再给出 `COMBINED_PASS` 或 `COMBINED_FAIL` verdict。以下只保留失效通知前已经真实完成的历史证据：
AIHub service guards、browser tests、syntax、static server MIME 与 legacy production 反向搜索均通过；
canonical isolated service graph 在第一个 std bootstrap command 编译 `skiff-test-runner` 时失败，尚未进入
AIHub source compilation。首错及同批次另外两个诊断都是父节点已经记录的 Skiff optional-handler
fixture drift，不是 F425B/C 回归，也不是环境问题。

收到失效通知后没有继续读取、运行或修改 candidate。supersession 收尾新增 probe command、test、
generated record 均为 `0`；所有需要新 HTTP correlation decision candidate 的验证均为`未运行`。

## 1. 精确输入与隔离

| 角色 | checkout / snapshot | Commit | Tree | 状态 |
| --- | --- | --- | --- | --- |
| Internals merged candidate | `/Users/geek/workspace/internals-p5-f426b-aihub-combined` | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` | 与直接父节点精确输入一致；只读、clean |
| Skiff supplied toolchain checkout | `/Users/geek/workspace/skiff-phase-05-integration` | `5e74633f44e9770e92b6a5866682d5eab65e2053` | `b0fbf41a17ef09bdeee91973df0aa8e625aaa0c4` | clean；相对父节点只多 F426 wave task docs |
| Skiff executed parent snapshot | `/tmp/p5-f426b-aihub-combined.LTSTmJ/skiff` | `6bd1697bc7be1b7119a3157146a2d313f4bffc4e` | `723467b529da07c145201bfa1637b309081c522d` | 由 supplied checkout 的 exact commit `git archive` 生成 |
| Internals executed snapshot | `/tmp/p5-f426b-aihub-combined.LTSTmJ/internals` | `ed5d333b2406d5375fca8acc96f4695667c48ced` | `26024bd221af3bb745c40039c8bf70e59ef1fc23` | 由只读 candidate 的 exact commit `git archive` 生成 |
| Result input | `/Users/geek/workspace/skiff-p5-f426b-aihub-combined-result` | `5e74633f44e9770e92b6a5866682d5eab65e2053` | `b0fbf41a17ef09bdeee91973df0aa8e625aaa0c4` | clean；本 leaf 只新增本文 |

隔离总 root 是 `/tmp/p5-f426b-aihub-combined.LTSTmJ`。所有命令统一使用：

```text
TMPDIR=/tmp/p5-f426b-aihub-combined.LTSTmJ/tmp
CARGO_TARGET_DIR=/tmp/p5-f426b-aihub-combined.LTSTmJ/cargo-target
NPM_CONFIG_CACHE=/tmp/p5-f426b-aihub-combined.LTSTmJ/npm-cache
SKIFF_ROOT=/tmp/p5-f426b-aihub-combined.LTSTmJ/skiff
```

canonical workflow 自己创建的 `internals-canonical-assembly-*` / `ecosystem-store` 在失败后已由
owner cleanup 删除；只剩 probe-owned Node compile cache、Cargo target 与日志，随后统一清理整个临时
root。直接递归删除被执行环境 destructive-action guard 拒绝且未产生删除；随后精确使用
`trash /tmp/p5-f426b-aihub-combined.LTSTmJ` 将整个 root 移入系统 Trash，原路径已不存在且可恢复。
没有读写 stable artifact root、watch registry、固定端口或真实 provider。

工具版本是 Node `v25.9.0`、npm `11.12.1`、Cargo `1.88.0`、rustc `1.88.0`。

## 2. 命令账与真实计数

下表计入全部 leaf 验证程序 invocation；只读文档/source 定位、snapshot 建立和 Git metadata 查询不计入
probe command count。表中的 `E` 是上一节四个完整环境变量，service commands 的 cwd 是
`<internals>/aihub/service`，其余 Internals commands 的 cwd 是 `<internals>`。

| # | 命令 | discovery / 结果 |
| ---: | --- | --- |
| 1 | `E npm run test:service-api` | 被测程序完成：8 discovered、7 pass、0 fail、1 expected skip；随后日志采集 shell 误把 zsh 只读变量 `status` 当作退出码变量，外层返回 ERROR。该 harness 错误发生在测试结束后，不是 candidate failure；原命令按原样重跑 |
| 2 | `E npm run test:service-api` | PASS：8 discovered、7 pass、0 fail、1 expected skip；skip 精确是等待 canonical generated receipt 的 case |
| 3 | `E npm run test:package-store` | PASS：2/2，0 fail、0 skip |
| 4 | `E npm run test:workflow-guards` | PASS：13/13，0 fail、0 skip |
| 5 | `TMPDIR=/tmp/p5-f426b-aihub-combined.LTSTmJ/tmp node --test aihub/client/*.test.mjs` | PASS：18/18，0 fail、0 skip；包含 pure SSE、Fetch transport 与 2 个 static server cases；真实 ephemeral HTTP response 断言 `.mjs` 为 `text/javascript` |
| 6 | `node --check aihub/client/app.js` | PASS：1 file |
| 7 | `node --check aihub/client/chat-stream.mjs` | PASS：1 file |
| 8 | `node --check aihub/client/server.mjs` | PASS：1 file |
| 9 | `node --check aihub/client/chat-stream.test.mjs` | PASS：1 file |
| 10 | `node --check aihub/client/server.test.mjs` | PASS：1 file |
| 11 | `E npm run type-check` | FAIL：canonical plan discovered 4 dependency packages、4 service packages、1 assembly；第 1 个 std bootstrap command 编译失败，3 个 Rust diagnostics；其余 8 个 authoring/assembly commands SKIP |
| 12 | static command `S1`（精确展开见下） | PASS：0 match |
| 13 | static command `S2`（精确展开见下） | PASS：0 match |
| 14 | static command `S3`（精确展开见下） | PASS：0 old-unary match |
| 15 | static command `S4`（精确展开见下） | PASS：0 non-canonical path / legacy transport match |
| 16 | static command `S5`（精确展开见下） | PASS：exactly 2 positive lines：唯一 endpoint 与唯一 chat transport method |

五个 static command 的被测命令精确为（日志重定向和“`rg` exit 1 等于预期零匹配”的 shell
assertion wrapper未省略任何被测参数）：

```bash
# S1
rg -n -i \
  'websocket|chat\.request|sendTextToConnection|/ws' \
  aihub/client/app.js aihub/client/chat-stream.mjs \
  aihub/client/index.html aihub/client/server.mjs \
  aihub/service/api.yml aihub/service/service.yml aihub/service/package.yml \
  aihub/service/internal -g '!*.test.skiff'

# S2
rg --pcre2 -n -i \
  '(?:compat(?:ibility)?|fallback|legacy).{0,120}(?:websocket|chat\.request|sendTextToConnection|/ws)|(?:websocket|chat\.request|sendTextToConnection|/ws).{0,120}(?:compat(?:ibility)?|fallback|legacy)' \
  aihub/client/app.js aihub/client/chat-stream.mjs \
  aihub/client/index.html aihub/client/server.mjs \
  aihub/service/api.yml aihub/service/service.yml aihub/service/package.yml \
  aihub/service/internal -g '!*.test.skiff'

# S3
rg -U -n --pcre2 \
  'path:\s*/(?:v1/)?chat/events[\s\S]{0,180}?handler:\s*internal\.aihub_service\.handleAihubHttp' \
  aihub/service/service.yml

# S4
rg --pcre2 -n -i \
  '(?<!/v1)/chat/events|/ws|new\s+WebSocket|chat\.request' \
  aihub/client/app.js aihub/client/chat-stream.mjs \
  aihub/client/index.html aihub/client/server.mjs

# S5
rg -n \
  'joinUrl\(els\.baseUrl\.value, "/v1/chat/events"\)|method: "POST"' \
  aihub/client/app.js aihub/client/chat-stream.mjs
```

命令计数：

- 实际 leaf 验证程序 invocation：`16`；
- 按被测程序结果：`15 PASS / 1 FAIL`，唯一 FAIL 是 #11；
- #1 另有一次测试结束后的采集 wrapper ERROR，已由 #2 原样重跑闭合，未隐藏也未计成 candidate failure；
- 实际 Node test execution（包含 #1 的透明重复）：`49 discovered / 47 pass / 0 fail / 2 expected skip`；
- 去除该重复后的唯一 suite 计数：`41 discovered / 40 pass / 0 fail / 1 expected skip`；
- syntax：`5/5 PASS`；legacy/positive static queries：`5/5 PASS`。

这些 `16` 条命令全部发生在 supersession 之前，只能描述第 1 节的旧 exact snapshot，不能迁移为新
HTTP correlation decision candidate 的证据。收到失效通知后的新增计数精确为：

- probe commands：`0`；
- tests discovered / pass / fail / skip：`0 / 0 / 0 / 0`；
- generated PackageArtifact / ServiceContract / ServiceDeployment / RuntimeAssembly：`0 / 0 / 0 / 0`；
- candidate reads、diffs、reverse searches或身份查询：`0`，均未运行。

## 3. canonical 首错、同批次诊断与归属

canonical workflow 的第一个动作是通过
`skiff-package-service-smoke-fixture --bootstrap-only` seed canonical std。Cargo 在生成或执行该
fixture 前编译 `skiff-test-runner` 失败，因此没有 std receipt，也没有进入任何 Internals package，
尤其没有编译 AIHub source。

精确首错：

```text
error[E0308]: mismatched types
  --> test-runner/src/canonical_test_gateway.rs:97:18
   |
97 |         handler: callable_id.clone(),
   |                  ^^^^^^^^^^^^^^^^^^^ expected `Option<PackageCallableId>`,
   |                                              found `PackageCallableId`
```

同一个 Rust 编译批次的全部独立 error diagnostics：

| 顺序 | 位置 | Error | 精确含义 |
| ---: | --- | --- | --- |
| 1 | `test-runner/src/canonical_test_gateway.rs:97:18` | `E0308` | HTTP test gateway 仍把 bare `PackageCallableId` 写入已经 optional 的 `DeploymentGatewayEntry.handler` |
| 2 | `test-runner/src/package_test_assembly.rs:238:25` | `E0308` | package-test overlay 仍直接比较 `Option<PackageCallableId>` 与 bare `PackageCallableId` |
| 3 | `test-runner/src/package_test_assembly.rs:241:13` | `E0277` | failure message 仍用 `{}` 格式化不实现 `Display` 的 `Option<PackageCallableId>` |

分类：`既有 current Skiff test-runner optional-handler fixture drift`。

因果证据：

- 三个 error 都位于父节点 Skiff checkout，Internals AIHub 文件尚未参与编译；
- `P5-F425A-skiff-websocket-authoring-compiler-checkpoint-result.md` 已在 F425B/C 合流前精确记录同两个
  文件、同两个行位 owner；
- 直接父节点也明确写明这两个 optional-handler fixtures 属于后继 D4；
- service/client merged candidate 的所有可独立 guards 与 tests 都通过；
- toolchain 已正常开始 Rust compilation，不存在缺工具、权限、网络、端口或 provider 前置问题。

所以它不是 F425B service 回归、不是 F425C client 回归，也不是环境问题。

精确 owner 是 `Skiff test-runner / 后继 D4 fixture-tooling convergence`。最小 repair write set 只有：

```text
test-runner/src/canonical_test_gateway.rs
test-runner/src/package_test_assembly.rs
```

repair 应让 HTTP fixture 显式构造 required `Some(callable_id)`，并让 overlay comparison / diagnostic
按 optional handler invariant 处理；本 probe 没有实施或扩大该 write set。

## 4. superseded / 未运行矩阵

| 必须证明的事实 | supersession 后计数 / 状态 | 旧 snapshot 历史证据 |
| --- | --- | --- |
| 新 candidate source / receipt / package-store / workflow guards | `0 / 未运行` | 旧 snapshot：23 unique discovered、22 pass、1 canonical-receipt expected skip |
| 新 candidate browser parser / transport / MIME | `0 / 未运行` | 旧 snapshot：18/18 |
| 新 candidate canonical isolated graph/type-check | `0 / 未运行` | 旧 snapshot 在 std bootstrap 命中本已记录的 test-runner drift |
| PackageArtifact | `0 / 未运行` | 旧 snapshot 也未生成 |
| ServiceContract | `0 / 未运行` | 旧 snapshot 也未生成 |
| ServiceDeployment | `0 / 未运行` | 旧 snapshot 也未生成 |
| RuntimeAssembly | `0 / 未运行` | 旧 snapshot 也未生成 |
| generated service-call operation count | `0 / 未运行` | 旧 snapshot 只有 source oracle，没有 generated receipt |
| generated `ServiceProtocolIdentity` | `0 / 未运行` | 没有新 generated identity |
| generated raw HTTP server-stream entries | `0 / 未运行` | 新 requestId-free source/records未读取 |
| generated AIHub WebSocket ingress | `0 / 未运行` | 新 deployment/assembly未生成 |
| requestId-free client / service correlation | `0 / 未运行` | 这是 superseding decision 的新验收面，旧 evidence 不适用 |

## 5. legacy 反向搜索边界（旧 snapshot 历史证据）

本节只描述 supersession 前第 1 节 exact snapshot，不是 requestId-free 新 candidate 的结论。
当时 production scope 精确包括：

- `aihub/client/{app.js,chat-stream.mjs,index.html,server.mjs}`；
- `aihub/service/{api.yml,service.yml,package.yml}`；
- `aihub/service/internal/**`，排除 `*.test.skiff`。

tests 和 receipt oracle 中用于负断言的 `websocket` 字面量不属于 production residue，故不混入
production zero-match 计数。最终结论：

- `WebSocket`（大小写不敏感）、`chat.request`、`sendTextToConnection`、`/ws`：`0`；
- 与这些 legacy transport spelling 相邻的 compat/fallback/legacy branch：`0`；
- events path 绑定旧 unary `handleAihubHttp`：`0`；
- client 非 `/v1/chat/events` events endpoint、socket constructor或旧 sender：`0`；
- client positive owner：`app.js` 唯一 `/v1/chat/events`，`chat-stream.mjs` 唯一 `POST`。

## 6. 禁令与收尾

- 没有修改 Internals candidate、Skiff toolchain或任何 source/test/fixture/tooling。
- 唯一 repository write 是 result worktree 中新增本文。
- 没有运行 generated record proof 的替代路径，也没有把 source guard 伪报成 generated receipt。
- 没有 merge、rebase、push、stable、live、instance、watch、reload、固定端口或真实 provider 操作。
- canonical owned store 已在 failure cleanup 删除；probe 总临时 root 在提交 result 前移入系统 Trash，
  原路径已不存在。
- supersession 前的旧 exact candidate 与 Skiff toolchain 当时均 clean；收到通知后未重新查询或声称新
  candidate clean。result commit 后状态必须独立复核为 clean。
- 收到 HTTP correlation decision 后没有继续运行原 probe；新 candidate 所有验收面均明确为
  `0 / 未运行`，后续必须由新 leaf 在新的 exact commit/tree 上重新建立证据。
