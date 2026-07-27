# P5-F440H External manifest strict DTO / compiler checkpoint

状态：Ready。确定性 shared leaf；对应 F440A 冻结 DAG 的 **M0**。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`
- `P5-F440A-external-manifest-owner-audit-result.md`
- `doc/reference/service-yml.md`

需要细节时只沿上述父文档引用向上读取。不得把历史 task/result 当成新的 schema owner。

精确实现输入：

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `a829bde6d250cd348a28f25c6246de6cbed2df9e` | `7d875d29a0b00fb93c618f4ff08ec2e381c11d60` |

## 目标

完成 external manifest 的 source trust boundary 与 compiler typed projection：

1. `service.yml` 的 strict DTO 只保留 `id`、`kind`、`serviceCalls`。
2. 可选 `http.yml` 是顶层 HTTP entry mapping；可选 `websocket.yml` 是单个 WebSocket entry。
3. `websocket.yml.jsonRpc` 的 declared method 被投影为独立 typed gateway entry。
4. Package compile input、PackageArtifact 与 ServiceContract 不读取 external manifest。
5. generated deployment revision 显式纳入已校验的 service/http/websocket authoring；timeout 仍只来自
   `config.<profile>.yml`。

本 leaf 只拥有 DTO、reader 与 compiler producer。Artifact identity/deployment follower 是下一节点 M1；
Router/Runtime broker、真实 service root 和 tooling migration均不在本任务。

## 唯一写集

Production：

- `artifact-model/src/ecosystem_authoring.rs`
- `artifact-model/src/gateway.rs`
- `artifact-model/src/deployment.rs`
- `artifact-model/src/schema.rs`
- `artifact-model/src/lib.rs`
- `compiler/input/src/service_config.rs`及其直接 export
- `compiler/driver/authoring.rs`
- `compiler/driver/generated_deployment.rs`
- `compiler/driver/http_gateway_projection/**`
- `compiler/driver/websocket_gateway_projection.rs`
- `compiler/driver/pipeline/mod.rs`
- `compiler/driver/lib.rs`

Tests：

- 上述 crate 的内联 unit tests
- `compiler/tests/http_gateway_projection.rs`
- `compiler/tests/websocket_ingress.rs`
- `compiler/tests/generated_service_deployment.rs`

另可新增本 leaf result。禁止修改 artifact-identity、deployment crate、Router、Runtime、scripts、
test-runner、fixture/service root、其它 task/result 或权威设计。不得派子 agent。

## 严格 authoring 合同

### `service.yml`

- `ServiceManifestAuthoring` 只拥有 `id`、`kind`、`service_calls`。
- `http`、`websocket`、`timeout` 以及任何 unknown field 立即拒绝；不保留 alias、dual-read 或迁移开关。
- 既有 id、kind、serviceCalls 的规范化与重复校验保持。

### `http.yml`

- 独立 document DTO；文件顶层直接是
  `GatewayEntryKey -> HttpGatewayEntryAuthoring`，不接受 `http:`、`routes:` 或 `entries:` wrapper。
- `{}` 合法；空文件/null/scalar/list 非法。
- duplicate entry key不能被 YAML map 静默覆盖。
- 保留并迁移现有 HTTP strict validation：selector唯一、method/path/host、kind、handler/guard/pre、
  adapterArgs和unknown/missing/duplicate field。

### `websocket.yml`

- 独立 singleton document DTO，顶层只允许 `path`、可选 `connect`、可选 `jsonRpc`。
- `path` 必填且 path-only document 合法；空文件/null/scalar/list/multi-entry wrapper非法。
- `connect: null` 非法；connect合同保持现有严格校验。
- `jsonRpc` 是可省略或空的 mapping。每个 entry 只允许 `method`、`handler`、`adapterArgs`：
  - key唯一，method非空、唯一，且不得以 `$/` 开头；
  - handler是当前 package source selector；
  - 必须且只能绑定一次完整 `websocket.jsonRpcParams`；
  - 可另绑定一次 `websocket.connectionId` 和一次 `websocket.businessIdentity`；
  - 不接受 transport id、raw frame、字段路径、guard、pre、receive、message、operation、event fallback、
    notification handler或手写 schema；
  - 所有参数名唯一，source phase正确。
- duplicate JSON-RPC key、method、handler field或top-level field都必须 fail closed。

## Root reader 合同

- `compiler/input/src/service_config.rs` 是三份 YAML 的唯一读取边界，新增
  `HTTP_CONFIG_FILE` / `WEBSOCKET_CONFIG_FILE` 及独立 reader。
- `ServicePackageRoot` 分别保存 `service`、可选 `http`、可选 `websocket`；不得把 external document
  重新塞回 `ServiceManifestAuthoring`。
- external file存在时必须是 regular file，而且同 root 必须有 regular
  `package.yml`、`api.yml`、`service.yml`。
- `compiler/driver/authoring.rs` 在进入 package-only 分支前盘点 external control files：
  ordinary package、external-only root或缺少合法 service/API 的 root必须 terminal 失败，不能静默忽略。
- `PackageCompileInput` / `PackageSourceInput` 不增加 external 字段；Package source/resource graph不读取它们。
- config profile继续独立读取；`service.yml.timeout` 不得回流。

## Typed projection / artifact vocabulary

- HTTP projection只接收独立 typed HTTP document，不再接收整个 service manifest。
- WebSocket connect projection只接收独立 typed WebSocket document。
- 每个 declared JSON-RPC method产生独立 gateway entry，拥有稳定 entry key、external method selector、
  linked handler signature、adapter plan与entry-local external schema。
- Artifact model加入第一版所需的 closed vocabulary：
  - JSON-RPC WebSocket adapter kind；
  - `websocket.jsonRpcParams`与`websocket.businessIdentity` source；
  - 与 connect 分离的 JSON-RPC protocol surface，包含 method/profile所需的 canonical shape。
- JSON-RPC params按linked handler parameter type投影；return只能unary，`void`编码语义为 `null`。
  `Stream<T>`、generic handler、source/type不匹配在 compiler projection阶段拒绝。
- `websocket.connectionId`沿用现有source；businessIdentity必须使用权威设计指定类型。
- 不实现 frame parsing、request id、pending registry、取消、Router/Runtime dispatch或JSON-RPC wire error。

## Generated deployment

- `GeneratedServiceDeploymentInput` 显式分开 service/http/websocket authoring。
- gateway entry和ingress binding由独立 documents生成。
- `generated_revision` 纳入三份已校验 typed authoring，因此 external manifest变化影响
  deployment revision。
- `DeploymentPolicy.timeoutMs` 只从 profile 的 scalar `timeout`读取；service/http/websocket均不拥有
  timeout。
- 本 leaf只生成新的 typed input，不实现 M1 的 identity normalization/validation。

## 测试先行与证据

先提交或记录真实 red，至少固定：

1. `service_manifest_rejects_inline_external_fields`：旧实现仍接受内联字段。
2. `reads_split_external_manifests`：旧 reader没有独立 DTO/字段。
3. HTTP `{}` 正例与 null/scalar/list/wrapper/duplicate key负例。
4. WebSocket path-only正例与 empty/null/scalar/list/wrapper/null-connect/duplicate field负例。
5. JSON-RPC key/method/source/signature/return strict matrix，包含 duplicate method、`$/`、缺少或重复
   params、transport id、stream return。
6. root inventory拒绝 external-only、ordinary package + external和缺少 API/service。
7. package compile input不含 external manifest；真实文件 mutation 后 PackageArtifact与
   ServiceContract exact bytes不变，而 generated deployment input/revision或gateway projection变化。

Focused commands：

```bash
cargo test -p skiff-artifact-model ecosystem_authoring
cargo test -p skiff-compiler-input service_config
cargo test -p skiff-compiler --test http_gateway_projection --test websocket_ingress --test generated_service_deployment
```

再运行受影响 crate check、`cargo fmt --all -- --check`、`git diff --check`，并反向搜索：

```bash
rg -n 'service\.(http|websocket)|ServiceManifestAuthoring.*(http|websocket|timeout)' artifact-model compiler
```

production中不得保留旧 ownership；命名清楚的negative test/source text fixture可列为允许命中。

## 与 M1 的交接

新增 closed enum/schema 后，artifact-identity或deployment follower出现 exhaustive-match /
validation compile failure属于预期 M1 blocker。不得越界修复，也不得用 wildcard、unknown、兼容 alias
掩盖。Result必须逐项列出：

- 已通过的 M0-owned tests；
- 因 M1 owner尚未迁移而无法运行的精确命令、文件和错误；
- schema marker/version是否需要 M1刷新及原因；
- M1需要消费的每个新 variant/field。

如果失败要求修改本任务未列出的 authoring/compiler owner，或公共语义仍不明确，停止并返回
`TASK_SCOPE_EXPANDED`。已知 M1 follower blocker不算 scope expansion。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440h-external-manifest-compiler`
- branch：`codex/p5-f440h-external-manifest-compiler`
- result：`P5-F440H-external-manifest-strict-dto-compiler-checkpoint-result.md`

implementation 与 result 分开提交；返回 commit/tree、red/green计数、reverse-search分类和clean状态。
不 merge/rebase/push。完成后保留 worktree，供主 agent 验收并在同一 shared branch上调度 M1。
