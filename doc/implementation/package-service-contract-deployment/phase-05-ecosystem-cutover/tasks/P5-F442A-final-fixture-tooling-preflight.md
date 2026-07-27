# P5-F442A Final fixture/tooling closeout preflight

状态：Ready。只读审计；在Router WebSocket RPC production闭合后，确定进入combined前剩余
fixture、test-runner、checker与README的准确清理DAG。

## 直接父节点

- `P5-F440Z3D-current-gateway-entry-wire-v2-hard-cut-result.md`
- `P5-F440Z3E-router-websocket-rpc-gateway-integration-resume-result.md`
- `P5-F441P-obsolete-live-case-removal-result.md`
- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`

父节点已经完成current GatewayEntry v2 wire、真实双向WebSocket JSON-RPC Gateway/server接线和
obsolete live case删除。Z3D仍记录两个明确基线阻塞：

- `runtime/package-test/tests/support/mod.rs` 两个initializer缺
  `collection_name_mapping`；
- runtime wire verifier进入legacy request fixture后遇到
  `ServiceProtocolIdentity v3`。

## 目标

只读确认combined前所有剩余**当前正例**与工具/说明是否仍使用过期语义，并拆成最少的、写集互斥的
实现节点。重点不是统计字符串，而是判断真实consumer是否会失败。

必须审计：

1. `cross-system-fixtures/package-service-ecosystem/`
   - GatewayEntry v2、ServiceProtocol v5、DeploymentArtifact v3等current identity；
   - obsolete WebSocket `receive`/route/automatic response/业务可见requestId；
   - current JSON-RPC requestId、runtime frame requestId与generation lifecycle requestId必须保留；
   - cancellation current terminal与旧public `CancelError`区别。
2. `runtime/package-test/`、`runtime/host/`与`test-runner/`
   - Z3D两个support initializer；
   - current-positive旧identity/golden；
   -新测试service写法、profile/target environment、removed test-doubles/live cases；
   -哪些 `publication` 只是内部尚有语义的source/resource owner，哪些是过时文档/fixture。
3. `scripts/check-skiff-source-layout.mjs`及直接相关checker/tests
   - current builtin/ActorRef/CancelError/InternalError/std websocket/file/http surface；
   - checker是否仍要求已经删除或遗漏current surface。
4. `router/README.md`、`runtime/README.md`及直接相关phase文档
   - external `http.yml`/`websocket.yml` owner；
   -无raw receive、无business routing、JSON-RPC双向request、notification/cancel；
   - `connection.send` downlink与RPC observed writer；
   - current identity generation；
   - publication术语若没有额外current信息，列为可删除文档，不做机械全文重命名。

不得把故意的stale-generation negative、内部request correlation或历史result证据误判为要删除。

## 允许的只读探针

使用共享Cargo target，可运行：

```bash
node scripts/check-skiff-source-layout.mjs
node cross-system-fixtures/package-service-ecosystem/verify.mjs \
  --runtime-wire-self-test
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-package-test --test package_artifact \
  entrypoint_validation_rejects_non_exact_gateway_facts
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-runtime-host --lib
CARGO_TARGET_DIR=/Users/geek/workspace/skiff-phase-05-integration/build/cargo-target \
  cargo test -p skiff-test-runner --test package_service_contract_deployment
```

若某命令预计超过审计窗口，可先list/no-run或执行最小filter，并在result记录未运行原因。不得启动stable、
watch、MongoDB、外部network、live或完整workspace suite。

## 允许读取范围

- 上述fixture/runtime/test-runner/scripts/README与其直接consumer
- `std/`、`prelude/`、compiler builtin registry只用于核对current surface
- 直接父任务/result

只允许新增：

`P5-F442A-final-fixture-tooling-preflight-result.md`

禁止修改production、test、fixture、checker、README或其它task/result；不得派子Agent。

## Result必须给出

### 1. 失败/过期矩阵

| Surface | Current owner/shape | Stale path | Earliest direct failure | Classification |
| --- | --- | --- | --- | --- |

Classification只能是：

- `BLOCKING_FIXTURE`
- `BLOCKING_TOOL`
- `STALE_DOCUMENTATION`
- `DELIBERATE_NEGATIVE`
- `CURRENT_INTERNAL_TERM`
- `NON_BLOCKING_FOLLOW_UP`

### 2. requestId / receive / cancellation分类

分别列出：

- 必须保留的transport correlation id；
- 必须隐藏于业务层的JSON-RPC id；
- 必须删除的旧业务DTO/route receive id；
- obsolete receive/route/automatic response fixture；
- internal cancellation signal/control与已删除public error surface。

### 3. 最小实现DAG

给出最少节点、依赖、精确写集、聚焦命令与可并行关系。优先拆成互不重叠的：

- Rust/test-runner fixture owner；
- cross-system corpus/verifier owner；
- checker/README owner。

若多个表面必须共享一个canonical生成物或checker，先列共享checkpoint，不能让consumer各自修。

### 4. combined入口

明确完成哪些节点后可进入cheap combined，哪些只是non-blocking文档清理；不得把README完美化设为
production gate，除非README/checker本身是仓库验收要求。

## 停止与交付

20分钟内形成单一明确DAG。若探针暴露新的公共契约或production owner，返回
`TASK_SCOPE_EXPANDED`并列证据；不得顺手实现。

- worktree：`/Users/geek/workspace/skiff-p5-f442a-fixture-tooling-preflight`
- branch：`codex/p5-f442a-fixture-tooling-preflight`

只提交result；不merge/rebase/push。
