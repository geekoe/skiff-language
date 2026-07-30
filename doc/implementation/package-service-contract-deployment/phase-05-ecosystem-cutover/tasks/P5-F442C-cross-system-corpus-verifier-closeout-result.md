# P5-F442C Cross-system corpus / verifier closeout result

状态：`PASS / CROSS_SYSTEM_CORPUS_CURRENT / CHEAP_COMBINED_X_CLOSED`。

本 leaf 已将 cross-system checkpoint 收敛到 current split external authoring，刷新 generic
legacy request 的外层 ServiceProtocol v5，并删除无 consumer 的 obsolete WebSocket response corpus。
current connect corpus 中 `receiveEvent` / old `websocketAdapter` rejection probes 保持不变。没有修改
Router/Rust production，没有增加 compatibility reader、dual path 或 fallback。

## 1. 基线、分支与提交

| 项目 | 值 |
| --- | --- |
| worktree | `/Users/geek/workspace/skiff-p5-f442c-cross-system-corpus` |
| branch | `codex/p5-f442c-cross-system-corpus` |
| implementation baseline | `0303fe5d` |
| task start HEAD | `2989ddf97391e97b99c3c1dd8c3d9468de0d28f7` |
| implementation commit | `fa3a8ed131702159a2348487d2b16b72c42ca7de` |
| result commit | 本文件独立提交，见 branch history |

`2989ddf9` 相对任务声明的 implementation baseline 只增加任务排期文档。Implementation 与本文
result 分开提交。

## 2. 真实 RED

worktree 初始没有 `router/node_modules`。第一次运行 verifier 因缺少 `yaml` 返回
`ERR_MODULE_NOT_FOUND`；该环境错误不计作 RED。按任务许可临时链接 integration worktree 的现有
Router dependencies 后，fixture implementation 修改前执行：

```bash
node cross-system-fixtures/package-service-ecosystem/verify.mjs \
  --runtime-wire-self-test
```

结果：exit `1`。current generic outer validator 在 verifier 的 legacy loop 中直接拒绝 v3 fixture：

```text
AssertionError: invalid request.start envelope:
serviceProtocolIdentity must be
skiff-service-protocol-v5:sha256:<64 lowercase hex>

false !== true
at runRuntimeWireSelfTest (.../verify.mjs:441:12)
```

这是真实父审计失败：corpus 尚未通过 current generic transport validator，所以测试没有到达下一条
“RuntimeAssembly typed validator 必须拒绝 legacy-only header”的断言。把该 header 的
ServiceProtocol v3 精确刷新为 v5 后，同一 loop 先通过 outer validator，再由 typed validator拒绝，
`--runtime-wire-self-test` 转绿。

## 3. Implementation

### 3.1 Current external authoring checkpoint

`checkpoint.json` 将过期的扁平 `authoringFields` 收敛为显式 `externalAuthoring`：

- `service.yml` fields 精确为 `id`、`serviceCalls`，test-only extension 精确为
  `{ kind: "test" }`；
- `http.yml` 顶层精确声明为 named entry mapping；
- `websocket.yml` 精确声明每 service 一个 physical entry，必填 `path`，可选 `connect` /
  `jsonRpc`；
- `config.<profile>.yml` 继续拥有 `timeout`，`service.yml` 不再拥有 HTTP、WebSocket 或 timeout；
- 原有 package service reference 与 assembly fields 只转换为同一显式结构，字段未改变。

`verify.mjs` 对以上完整 object 做 exact deep equality，不能再把旧 inline
`service.yml.http/websocket/timeout` 自证为正确。

### 3.2 Generic legacy request

`runtime-request-wire.json` 只把 `legacyRequestStartHeaders[0].serviceProtocolIdentity` 从 v3 更新为
v5。legacy request shape、build id、selector、trace、payload以及整个 legacy-only loop均保留；没有删除
generic branch，也没有修改 production decoder。

### 3.3 Obsolete response corpus

删除无 executable consumer 的
`cross-system-fixtures/package-service-ecosystem/runtime-websocket-response-wire.json`。该文件原有
272 行，包含 stale positive `webSocketReceive`、ServiceProtocol v2 与旧 gateway identity。current
connect response继续由 `runtime-websocket-connect-wire.json` 和 Router/Rust shared consumers拥有。

## 4. 规定 non-live 验证

| 命令 | 结果 |
| --- | --- |
| `node .../verify.mjs --self-test` | PASS：6 controls，79 raw cases |
| `node .../verify.mjs --combined-probe` | PASS：`activation-parity` |
| `node .../verify.mjs --runtime-wire-self-test` | PASS：6 activation frames、7 activation mutations、3 request headers、115 request mutations、19 raw cases、4 payload cases、1 equivalent pair、1 legacy header、6 store operations |
| 规定 Router `vitest list` | PASS：独立 `wc -l` 为 **164**，明确 non-zero |
| 规定 Router `vitest run` | PASS：3 files，164/164 tests |
| `cargo test -p skiff-runtime-transport runtime_assembly_request` | PASS：19 passed、75 filtered；附带 integration binary 0 executed、2 filtered |
| `git diff --check` | PASS |

Rust 命令使用任务规定的：

```text
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target
```

Router 规定三文件实际收集并执行 164 项，不以 wrapper 的零测试成功替代证据。测试时临时链接：

```text
router/node_modules ->
/Users/geek/workspace/skiff-phase-05-integration/router/node_modules
```

implementation 提交前已删除该链接；dependency tree 与 symlink 均未提交。

## 5. 反向搜索与自验收

| 任务条款 | 代码证据 | 验证证据 |
| --- | --- | --- |
| split external authoring | checkpoint `externalAuthoring` + verifier exact assertions | self-test PASS |
| timeout只归 profile | service fields无 timeout；profile fields含 timeout | exact deep equality PASS |
| generic legacy outer identity current | legacy header v5，shape与 loop保留 | runtime-wire self-test从真实 RED转绿 |
| obsolete response corpus删除 | 旧 JSON文件删除 | executable tree 文件名 consumer搜索为0 |
| 保留 current connect negatives | connect corpus仍有 named `legacy receive event` 与 `legacy websocket adapter` | Router list/run包含两项拒绝测试，164/164 |
| 不改 production / 不加兼容 | implementation commit只有任务规定4个 corpus/verifier路径 | `git diff --name-only`写集审计 |

反向搜索还确认：

- cross-system corpus 不再含该 generic fixture 的 v3 digest；
- checkpoint 不再含旧 `service.yml = id,http,websocket,timeout`；
- `runtime-websocket-connect-wire.json` 原样保留
  `websocketConnect.receiveEvent` 与 `{ websocketAdapter: { kind: "receive" } }` mutations；
- obsolete corpus 文件名只剩任务与历史 result 文档引用，没有 source/test/tool consumer。

## 6. 范围声明

Implementation 精确修改任务允许的四个路径：三个文件更新、一个文件删除。本文是唯一额外 result。
没有启动 stable instance、watch、MongoDB、server 或 live workload；没有访问 network；没有派生
sub-agent；没有 merge、rebase 或 push。
