# P5-F445H-I7-K Service-scoped ingress canonical checkpoint result

状态：

```text
PASS
K_COMPLETE = YES
CONSUMER_WAVE_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
C_REVALIDATION_BLOCKED_ON_CONSUMERS = YES
```

K已经建立D0后续并行consumer共同依赖的canonical检查点：`IngressSelector`不再包含Host；
RuntimeAssembly ingress identity由精确deployment与service-local selector共同组成；同一active assembly
拒绝相同service/version的多个deployment；相关deployment、assembly与runtime frame generations已经
hard cut。

K是实现检查点，不是纵向完成态。compiler、resolver/loader/linker、Router/Host request wire及跨系统fixture
必须在后续consumer wave迁移后，才能恢复全仓编译、完整transport套件与Relay/AIHub combined evidence。

## 1. Parent and exact identities

| 项 | 值 |
| --- | --- |
| direct task | `P5-F445H-I7-K-service-scoped-ingress-canonical.md` |
| direct parent | `P5-F445H-I7-D0-service-scoped-ingress-design-result.md` |
| baseline commit/tree | `46f75981767da95a884644d8610ac43d3c689934` / `59c2a667ccbd73f98e92efa3eb81347918938973` |
| implementation commit/tree | `005f8ea058d25e1009e39ab9926c14e7acdf2460` / `9577c59079cc659c7b922b3f864f36973e153e20` |
| branch | `codex/p5-f445h-i7-k-ingress-identity` |
| worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-k-ingress-identity` |
| integration owner | `/root/phase05_integration_steward` |

最终result commit/tree在Git handoff中报告；result不能自引用自己的commit identity。

## 2. Canonical implementation

### Selector and scoped key

- `artifact-model/src/deployment.rs`中的`IngressSelector`现在只有
  `protocol + method + path`，继续使用`deny_unknown_fields`；
- 带旧`host`字段的selector JSON反序列化失败；
- `ServiceIngressKey`精确持有`ServiceDeploymentRef + IngressSelector`，可排序、hash与严格serde；
- `GatewayIngressBinding::service_ingress_key()`从binding唯一字段导出key，不复制第二份identity状态；
- assembly validation按该scoped key判重，因此不同service共享相同method/path合法，同一精确deployment
  重复selector失败。

### Active assembly coordinate

`validate_deployments`现在按`serviceId + contractVersion`建立active coordinate，第二个不同revision或
artifact identity会fail closed。精确重复仍先由原有exact-set检查拒绝。

### Hard-cut generations

| Owner | Current generation |
| --- | --- |
| ServiceDeploymentInput | `skiff-service-deployment-input-v5` |
| ServiceDeployment schema | `skiff-service-deployment-v4` |
| DeploymentArtifact identity marker | `skiff-deployment-artifact-identity-v4` |
| DeploymentArtifact identity prefix | `skiff-deployment-artifact-v4:sha256` |
| RuntimeAssembly schema | `skiff-runtime-assembly-v3` |
| RuntimeAssembly identity marker | `skiff-runtime-assembly-identity-v3` |
| RuntimeAssembly identity prefix | `skiff-runtime-assembly-v3:sha256` |
| Rust/TypeScript runtime frame | `skiff-runtime-frame-v2` |

GatewayEntryIdentity/GatewayEntry保持v2；ServiceContract/ServiceProtocol、Package artifact/build/local ABI/schema
与WebSocketEntryId均未改代际。

### Minimal mechanical closure

为了让K自己的runtime transport聚焦证据可执行，仅在
`runtime/transport/src/ingress_selector.rs`删除已不存在的Host构造与辅助函数，并在
`runtime/transport/src/request_mapper.rs`删除对应test读取。URL仍必须具有合法Host，原始URL继续作为HTTP
业务metadata；这些修改没有实现header deployment选择、request frame exact deployment或Host/Router路由。

K还只刷新了被本检查点聚焦测试直接消费的activation/bootstrap/actor identity与actor-method parity
fixtures。完整runtimeAssembly request、WebSocket lifecycle及跨系统fixture统一留给后续consumer/fixture
节点，避免在canonical checkpoint提前定义其最终wire shape。

## 3. Evidence

| 层级 | 命令 | 代码状态 | 结果 | 覆盖 |
| --- | --- | --- | --- | --- |
| canonical model | `cargo test -p skiff-artifact-model` | implementation commit | PASS 180/180 | strict selector wire、schema generations、activation ref fixtures |
| identity/validation | `cargo test -p skiff-artifact-identity` | implementation commit | PASS 136/136 + CLI 8/8 | scoped collision、multi-revision拒绝、deployment/assembly identity hard cut |
| runtime frame | `cargo test -p skiff-runtime-transport protocol::tests` | implementation commit | PASS 31/31 | v2 common frame owner、v1 bootstrap拒绝、bootstrap parity |
| actor parity | `cargo test -p skiff-runtime-transport actor_method::tests::shared_rust_typescript_parity_corpus` | implementation commit | PASS 1/1 | current v2与previous-v1 negative |
| format | `cargo fmt --all -- --check` | implementation commit | PASS | Rust formatting |
| whitespace | `git diff --check` | implementation commit | PASS | whitespace/conflict markers |
| Router static owner | `router/src/protocol/envelope.ts` inspection | implementation commit | PASS | TypeScript common constant exact v2 |
| Router type-check environment | `pnpm --filter @skiff/router type-check` | worktree | NOT RUNNABLE：`tsc`不存在且worktree无`node_modules` | 未安装依赖、未访问network |

一次非gate的完整transport诊断在implementation commit上得到79/95，16个失败都映射到明确后续consumer：
旧activation binary golden、actor/spawn test identity、request error文本、runtimeAssembly request/response
golden与WebSocket lifecycle corpus。该命令不是K完成标准，结果用于下游一次性fixture/consumer wave，不应在
K重复修补。

## 4. Downstream handoff

K合并后可以并行启动：

1. compiler/authoring：从`http.yml`/`websocket.yml`与projection删除Host route字段，输出v5/v4；
2. assembly/loader/linker：resolver使用service-scoped key，刷新v4/v3 artifact读取与激活路径；
3. Router/Runtime/Host wire：严格解析service/version headers，frame v2携带exact deployment，
   ingress routing删除Host并完成WebSocket deployment pin；
4. fixture/golden：在consumer shape冻结后统一刷新activation、runtime request、WebSocket lifecycle、
   Router protocol与cross-system corpora；
5. join：Relay与AIHub共享`GET /v1/models`，不同header坐标选择不同deployment并执行相同exact
   deployment；全部负例重新建立。

当前明确残留包括：

- `artifact-model/src/ecosystem_authoring.rs`与compiler projection仍拥有HTTP Host authoring；
- `deployment/src/assembly/resolver.rs`仍使用裸selector map；
- runtime loader/Host/request与Router gateway仍读取Host selector；
- Router specialized actor-owner路径仍有hardcoded runtime-frame-v1；
- runtimeAssembly request v1 fixtures仍含Host与旧assembly identity。

这些都是D0/X1已经划分的consumer owner，不是K遗漏，也不能用兼容字段或dual-read在K内掩盖。

```text
K_COMPLETE = YES
CONSUMER_WAVE_UNBLOCKED = YES
DECISION_REQUIRED = NO
BLOCKING_ISSUES = 0
```
