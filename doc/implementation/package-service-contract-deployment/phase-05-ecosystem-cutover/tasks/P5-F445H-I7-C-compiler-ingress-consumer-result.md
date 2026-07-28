# P5-F445H-I7-C Compiler ingress consumer result

状态：

```text
PASS
C_COMPILER_COMPLETE = YES
CONSUMER_JOIN_READY = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```

C已经关闭K在compiler/authoring域留下的Host断链：`http.yml`不再声明、默认或规范化Host；
HTTP/WebSocket projection只生成service-local `protocol + method + path` selector；旧`host`
authoring严格失败。同一service仍按`method + path`拒绝重复HTTP route，不同service可分别声明同一路由。

## 1. Parent and exact identities

| 项 | 值 |
| --- | --- |
| direct task | `P5-F445H-I7-C-compiler-ingress-consumer.md` |
| direct parent | `P5-F445H-I7-K-service-scoped-ingress-canonical-result.md` |
| authority parent | `P5-F445H-I7-D0-service-scoped-ingress-design-result.md` |
| baseline commit/tree | `1a11328a241b5d177eb40885e294fe31d65a7240` / `ca1f7c2f040458df4275d00801eb0fc61046a1a8` |
| implementation commit/tree | `c5a035ed11dc8e928a1cb5f516b69ea566eddb25` / `bfb1bd6d952f73f22a363f13a4037e4fc8e0e86c` |
| validation baseline commit/tree | `b47dbcdffab19a176518a233acd6a6c584ddb796` / `42e0f65a79576e1438ce99d1a9bc8d227972fa03` |
| detached validation join commit/tree | `08199a8b3ef21c0b1f0ad6f9aaf1801061151cd3` / `65f478c5d9c6962fbbb3e6213c4fc581b868beda` |
| branch | `codex/p5-f445h-i7-c-compiler-ingress` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-c-compiler-ingress` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree在Git handoff中报告；result不能自引用自己的commit identity。

验证join只在从R+L checkpoint创建的detached临时worktree中cherry-pick C implementation，用于证明并行
consumer组合可编译和执行。临时worktree已经删除，R+L提交没有进入C分支。

## 2. Implementation

### Strict authoring hard cut

- `HttpGatewayEntryAuthoring`删除旧`host`字段和`*`默认值；
- DTO继续使用`deny_unknown_fields`，旧`host`明确返回`unknown field`，没有忽略、alias、fallback或
  dual-read；
- `service_config`删除Host校验和小写规范化；HTTP route只校验、规范化`method + path`；
- 同service的重复`method + path`继续失败；两个独立service root可各自接受同一
  `GET /v1/models`。

### Projection

- HTTP projection生成`IngressProtocol::Http + method + path`；
- WebSocket connection与JSON-RPC method projection生成
  `IngressProtocol::WebSocket + method? + path`；
- 两条projection均不再构造`IngressSelector.host`；
- generated deployment继续从canonical常量取得ServiceDeploymentInput v5，并在当前
  ServiceDeployment v4 / DeploymentArtifact v4 consumer链上通过聚焦生成测试。

### Fixtures and direct tests

- HTTP identity测试只用method/path变化证明selector属于deployment identity而不属于gateway identity；
- WebSocket测试删除旧`*` Host oracle，保留protocol/path/method精确断言；
- compiler-owned router WebSocket fixture删除旧Host authoring；
- 新增旧Host严格拒绝和两个service root共享同一路由的证据。

## 3. RED and GREEN evidence

### Real RED

在未修改production的baseline `1a11328a` detached临时worktree中，只加入
`http_document_rejects_legacy_host_field`断言：

```text
running 1 test
http_document_rejects_legacy_host_field ... FAILED
0 passed; 1 failed
```

失败原因是baseline仍成功反序列化带`host: api.example.com`的HTTP authoring，精确证明本任务要移除的旧
行为。该临时worktree已删除。

K baseline上的三条compiler命令还先被并行R+L owner的
`deployment/src/fixtures.rs`旧`IngressSelector.host`编译断链阻塞；C没有越界修改。R+L checkpoint
集成后，在精确validation join上重跑全部转绿。

### Final verification

| 层级 | 命令 | 结果 |
| --- | --- | --- |
| authoring/model | `cargo test --locked -p skiff-artifact-model` | PASS，180/180 |
| input production compile | `cargo check -p skiff-compiler-input --lib` | PASS |
| service config join | `cargo test --locked -p skiff-compiler-input service_config` | PASS，20/20 |
| HTTP projection join | `cargo test --locked -p skiff-compiler --test http_gateway_projection` | PASS，11/11 |
| WebSocket projection join | `cargo test --locked -p skiff-compiler --test websocket_ingress` | PASS，10/10 |
| focused dynamic total | 上述三条locked join tests | PASS，41/41 |
| formatting | `cargo fmt --all -- --check` | PASS |
| whitespace | `git diff --check` | PASS |
| write set | `git diff --name-only` against each exact baseline | PASS，仅本任务8个文件 |
| stale production search | `authoring.host`、`selector.host`、`default_http_host`、`validate_http_host` | PASS，0 |
| positive search | legacy Host rejection、per-service route、`method + path` selector | PASS |

编译输出中的unused/dead-code warning来自既有compiler source/runtime linker代码，本任务未修改对应owner，
不影响测试结果。

## 4. Actual write set

```text
artifact-model/src/ecosystem_authoring.rs
compiler/input/src/service_config.rs
compiler/driver/http_gateway_projection/mod.rs
compiler/driver/websocket_gateway_projection.rs
compiler/tests/http_gateway_projection.rs
compiler/tests/websocket_ingress.rs
compiler/tests/fixtures/router-websocket-fixture/http.yml
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-C-compiler-ingress-consumer.md
  P5-F445H-I7-C-compiler-ingress-consumer-result.md
```

这里的“8个文件”是implementation commit；result commit另修改task状态并新增本result。没有修改assembly
resolver/loader/linker、runtime Host/eval、Router TypeScript、canonical K owner、Internals或official
packages。

## 5. Handoff

C可以交由`/root/phase05_integration_steward`合入当前consumer candidate。合入后，统一fixture/golden与最终
join owner应继续验证：

- Relay与AIHub同时声明`GET /v1/models`；
- 不同service/version header选择不同deployment；
- 同service重复route、旧Host authoring和旧Host selector wire继续失败。

本任务没有运行stable/live/network/Mongo/OAuth/browser，没有push。

```text
C_COMPILER_COMPLETE = YES
CONSUMER_JOIN_READY = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
