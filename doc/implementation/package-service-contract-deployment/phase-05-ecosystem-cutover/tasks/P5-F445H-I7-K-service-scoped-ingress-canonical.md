# P5-F445H-I7-K Service-scoped ingress canonical checkpoint

状态：`IMPLEMENTED_PENDING_RESULT`。

## 1. Parent chain and DAG position

直接父节点：

- `P5-F445H-I7-D0-service-scoped-ingress-design-result.md`。

该result经D0 task、I7R readiness与Phase 05 DAG追溯到唯一架构事实源
`doc/architecture/package-service-contract-deployment.md`。D0已经冻结Host不参与Skiff Router路由、
service-scoped ingress key和本轮hard-cut generations。

```text
D0 authority cutover
  -> K canonical model/schema/identity/wire checkpoint
  -> compiler / assembly / Router+Runtime consumers
  -> Relay+AIHub combined revalidation
```

K只建立后续并行consumer共同依赖的canonical检查点，不把暂时断链的consumer迁移吞入本任务。

## 2. Frozen baseline and worktree

| 项 | 值 |
| --- | --- |
| baseline commit | `46f75981767da95a884644d8610ac43d3c689934` |
| baseline tree | `59c2a667ccbd73f98e92efa3eb81347918938973` |
| integration branch | `codex/package-service-phase-05` |
| leaf branch | `codex/p5-f445h-i7-k-ingress-identity` |
| leaf worktree | `/Users/geek/workspace/skiff-p5-f445h-i7-k-ingress-identity` |
| integration owner | `/root/phase05_integration_steward` |

零worktree只读预检确认：

- `IngressSelector` canonical owner是`artifact-model/src/deployment.rs`，现有字段为
  `protocol + host + method + path`；
- `RuntimeAssembly`已将deployment与selector同置于`GatewayIngressBinding`，但identity validation仍只按
  裸selector判重；K可在模型层用`ServiceDeploymentRef + IngressSelector`形成scoped key；
- 同一`serviceId + contractVersion`多revision的拒绝owner位于
  `artifact-identity/src/runtime_assembly/validation.rs`，可在K闭合；
- schema/identity常量分别位于`artifact-model`与`artifact-identity`；runtime frame canonical常量分别位于
  Rust transport和Router protocol；
- 删除`IngressSelector.host`会使compiler、resolver、loader、Host/Runtime与测试fixture暂时断链。它们属于
  K后续consumer wave；K不为保持全仓编译添加兼容字段或fallback。

## 3. Write ownership

K独占：

```text
artifact-model/src/deployment.rs
artifact-model/src/runtime_assembly.rs
artifact-model/src/schema.rs
artifact-model/src/activation_lexical.rs
artifact-identity/src/constants.rs
artifact-identity/src/deployment/**
artifact-identity/src/runtime_assembly/**
artifact-identity/src/tests/**
runtime/transport/src/protocol.rs
runtime/transport/src/protocol/tests.rs
runtime/transport/src/ingress_selector.rs
runtime/transport/src/request_mapper.rs
router/src/protocol/envelope.ts
doc/implementation/package-service-contract-deployment/phase-05-ecosystem-cutover/tasks/
  P5-F445H-I7-K-service-scoped-ingress-canonical.md
  P5-F445H-I7-K-service-scoped-ingress-canonical-result.md
```

后两项runtime transport文件只允许删除已不存在字段的constructor/test读取，不取得exact deployment或
Router路由职责。因果上直接的canonical fixture/golden只有在上述聚焦crate需要时才纳入。禁止修改compiler authoring、
deployment resolver/loader/linker、Router/Host routing consumer、Internals或official packages。

## 4. Required implementation

1. `IngressSelector`删除`host`，wire继续`deny_unknown_fields`；带旧`host`的selector反序列化失败。
2. 新增可排序、可hash、严格serde的scoped ingress key，精确表达
   `(ServiceDeploymentRef, IngressSelector)`；`GatewayIngressBinding`从自身唯一的deployment/selector字段
   导出该key，不复制第二份identity状态。
3. assembly validation按scoped key判重：不同deployment共享selector合法；同一deployment重复selector失败。
4. assembly validation拒绝同一`serviceId + contractVersion`的多个revision或artifact identity。
5. hard cut：
   - ServiceDeploymentInput v5；
   - ServiceDeployment v4；
   - DeploymentArtifact identity marker/prefix v4；
   - RuntimeAssembly schema/identity v3；
   - Rust/TypeScript runtime frame常量v2。
6. GatewayEntryIdentity/GatewayEntry v2、ServiceContract/Protocol、Package artifact/build/local ABI/schema与
   WebSocketEntryId保持不变。
7. 旧schema、旧identity prefix、旧Host selector和旧runtime frame必须被canonical decoder/validator拒绝。

## 5. Non-goals and temporary disconnect

- 不实现header解析、deployment选择、Router/Runtime exact deployment frame内容或WebSocket pin；
- 不修改http/websocket authoring、compiler projection或源配置；
- 不实现assembly resolver、loader/linker及Host execution consumer；
- 不刷新跨系统fixture或运行全仓gate；
- 不运行stable/live/network/Mongo/OAuth/browser，不push。

consumer因结构删除产生的编译错误是后续扇出的显式输入，不得在K中用optional host、serde alias、dual-read或
默认值掩盖。只有K自身crate内的constructor、validation test和canonical常量断言可机械闭合。

## 6. Verification and completion

K是高风险canonical checkpoint。开发自验收只运行：

```bash
cargo test -p skiff-artifact-model
cargo test -p skiff-artifact-identity
cargo test -p skiff-runtime-transport protocol::tests
cargo fmt --all -- --check
git diff --check
```

若runtime transport聚焦测试因下游旧constructor断链而无法编译，只保留frame常量与精确静态/单测证据，并在
result中列出断链owner；不扩张到consumer实现。完成时还需反向搜索：

- canonical `IngressSelector`无`host`；
-上述新版本常量精确且旧版本不再是canonical owner；
-未变版本保持原值；
-实际写集没有越过K owner。

提交implementation与result后，向`/root/phase05_integration_steward`报告branch、worktree、commit/tree、
写集、验证与后续断链清单；不自行写integration branch。
