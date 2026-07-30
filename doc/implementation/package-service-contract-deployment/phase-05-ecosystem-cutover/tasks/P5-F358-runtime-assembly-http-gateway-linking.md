# P5-F358 RuntimeAssembly HTTP gateway linking

状态：Ready（C3 shared prerequisite；只形成可供后续 consumer 使用的 linked gateway
checkpoint）。

## 直接父节点

- `P5-H36-external-ingress-implementation-dag.md`
- `P5-F348-external-ingress-runtime-router-audit-result.md`
- `P5-F355-deployment-http-gateway-model-result.md`
- `P5-F357-http-gateway-compiler-projection-result.md`

以上父节点沿引用链连接唯一权威设计。本任务只完成 H36 C3 中位于 transport、Host、Router 和
test-runner 之前的共享 Rust checkpoint；不得重新设计 HTTP authoring、gateway identity、
external schema、runtime codec、错误投影、超时执行责任、stream wire 或 WebSocket 业务消息入口。

## Exact base

- integration commit：`33cc73d2f3fc942cad6ca69ebdd3fd95d392fb9e`
- integration tree：`0b8642996048c2175300de03032beadc71347731`
- branch：`codex/package-service-phase-05`

本 checkpoint 已包含：

- Deployment v2 的 `gatewayEntries` 与 `selector -> GatewayEntryKey`；
- compiler 生成的 exact handler/pre/guard callable、adapter plan、HTTP protocol surface 与
  `GatewayEntryIdentity`；
- RuntimeAssembly、loader、linker、Host、Router 与 test-runner 对 nonempty gateway ingress
  的显式 fail-closed。

## DAG 位置与解除条件

```text
F357 compiler projection
  -> F358 RuntimeAssembly model / assembly / loader / linker
       -> shared Rust/TypeScript request protocol
          -> Host/runtime consumer
          -> Router consumer
          -> test-runner gateway execution
```

本任务完成后只形成“实现检查点”，不是预验收或稳定候选。它解除 request protocol、Host、
Router 和 test-runner consumer，但不证明任何 external request 已经可执行。

## 目标

把 Deployment 已经拥有的 HTTP gateway entry 精确接入 RuntimeAssembly，并在 runtime linker
得到不依赖 ServiceContract operation 的 linked entry：

```text
IngressSelector
  -> exact ServiceDeploymentRef + GatewayEntryKey + GatewayEntryIdentity
  -> exact DeploymentGatewayEntry
  -> exact implementation PackageArtifact
  -> linked handler/pre/guard target + PackageCallableSignature
  -> exact activation owner
```

External ingress 的任何一步都不得读取、生成或恢复 `ContractOperationId`。Internal
service-to-service operation graph 必须保持原样。

## 必须完成

### 1. RuntimeAssembly v2 gateway surface

用一个职责明确的 gateway ingress DTO 替换旧 `GlobalIngressBinding`。Canonical serialized
binding 必须且只能包含：

- `selector: IngressSelector`；
- `deployment: ServiceDeploymentRef`；
- `gatewayEntryKey: GatewayEntryKey`；
- `gatewayEntryIdentity: GatewayEntryIdentity`。

RuntimeAssembly 的 canonical field 使用 `gatewayIngress`，删除旧 `globalIngress` field 和
`GlobalIngressBinding` 类型；不保留 serde alias、default、dual read、conversion helper 或 legacy
fallback。

RuntimeAssembly schema、assembly identity schema marker 与 identity prefix 同步升级为 v2。Identity
preimage 包含完整 `gatewayIngress`，按既有 canonical unordered collection 规则归一化。以下变化必须
改变 assembly identity：

- selector；
- deployment exact ref；
- gateway entry key；
- gateway entry identity。

仅改变 binding 插入顺序不得改变 identity。旧 v1 schema、v1 identity prefix、旧
`globalIngress`、旧 contract/operation fields 和缺失 required `gatewayIngress` 必须严格拒绝。

Surface validation 至少保证：

- selector 全局唯一；
- deployment ref 属于 `resolvedDeployments`；
- key 与 identity 是各自严格 typed value；
- empty assembly 仍要求 empty gateway ingress；
- assembly-level validation 不伪造或猜测 deployment content。

### 2. Deployment assembly exact projection

删除 `GatewayIngressNotLinked` checkpoint。`resolve_runtime_assembly` 必须从每个 resolved
deployment 的 canonical `ingress` 与 `gatewayEntries` 生成 exact assembly binding：

- selector、deployment ref、key逐值保留；
- identity只读取 key 对应的 `DeploymentGatewayEntry.gatewayEntryIdentity`；
- 一个 entry 被多个 selector 引用时生成多个 selector binding，但仍指向同一 deployment/key/identity；
- zero gateway/zero ingress 保持合法；
- 跨 deployment selector collision fail closed；
- missing key、identity mismatch或任何无法形成 exact binding 的状态 fail closed，不静默省略。

生成顺序必须 canonical，且 RuntimeAssembly identity 在相同图上稳定。不要把 gateway entry
重新投影成 ServiceContract 或 operation binding。

### 3. Loader exact cross-object join

`RuntimeAssemblyLoader` 必须把 `gatewayIngress` 与 hydrated `ServiceDeployment`、
implementation `PackageArtifact` 做 exact join，并至少验证：

- assembly binding 恰好等于所有 hydrated deployment ingress 的 canonical union；不得缺失、
  多出或重复；
- binding deployment/key/identity 与对应 deployment entry 精确一致；
- selector 与 HTTP protocol surface 一致，当前 generation 不接受 WebSocket；
- handler、可选 pre、可选 guard 都存在于 deployment implementation package 的
  `callableLinks`；
- 每个 callable link 的 map key/nested ID/target 与 implementation artifact 精确一致；
- 每个 callable 都能从 exact local ABI callable symbol取得唯一 `PackageCallableSignature`；
- callable target 已由既有 hydrated file/code validation证明存在；
- entry 不能借用 dependency package callable、ServiceContract operation descriptor 或
  public/display path补全缺失事实。

F355/F357 已经拥有 protocol surface、adapter/source 与 compiler signature规则。本任务不得在
loader 复制 external schema projector、gateway identity hash或 runtime codec；loader只做
immutable cross-object exactness与后续 linking 所需事实闭合。

### 4. Runtime linker exact linked entry

新增职责单一的 linked gateway 类型。命名可以服从现有 Rust 模块风格，但候选必须能直接取得：

- owner `ServiceDeploymentRef` / activation；
- `GatewayEntryKey` 与 `GatewayEntryIdentity`；
- canonical `GatewayEntryProtocolSurface` 与 ordered `GatewayAdapterPlan`；
- handler 的 exact `PackageCallableId`、`OperationTargetRef` 与
  `PackageCallableSignature`；
- optional pre/guard 各自的 exact callable ID、target 与 signature。

同一 deployment/key 被多个 selector 使用时，不得重新解释或产生不同 linked entry。Candidate
应提供 selector 到 linked entry 的直接 typed lookup，并使后续 admission 能同时 exact-match
selector、entry identity 与 activation；具体可用共享 `Arc` 或规范化 entry index，不能把
ServiceContract lookup留给后续 consumer。

Linking 必须 fail closed：

- binding 指向 missing activation/deployment/entry；
- key 或 identity不一致；
- callable missing、wrong package、wrong nested ID或target错配；
- callable signature不存在、歧义或与 exact callable ID不一致；
- selector collision；
- linked ingress集合不等于 assembly declaration。

`LinkedContractOperation`、`ServiceContractStore`、operation target tables、service binding 和
activation-relative service call继续只服务 internal service calls；不得为了 gateway cutover
删除或改名。

### 5. 当前 consumer 的显式边界

本 leaf 不得让请求执行“看起来可用”。以下仍由后续独立任务拥有：

- Rust/TypeScript request routing wire 中
  `contractOperationId -> gatewayEntryIdentity/entry reference`；
- Host admission、request/eval、adapter codec、response codec与 telemetry target；
- Router RuntimeAssembly snapshot、HTTP dispatch、timeout、cancel与stream backpressure；
- test-runner 合成 deployment/entrypoint 与真实执行；
- cross-system request corpus与真实 Router/Host probes；
- WebSocket connection/business entry。

若直接 Rust consumer 因 RuntimeAssembly v2 不再编译，只允许在本任务 owner范围内更新
artifact-model、artifact-identity、deployment、runtime loader/linker 的构造器、fixture与聚焦测试。
不要修改 Host、request、transport、Router、test-runner或跨系统 wire来制造 workspace 绿色；在
result中列出明确被解除的下游编译迁移点。

### 6. 不受待决语义影响

本任务只携带 F357 已有 `GatewayDispatchMode` 与 Deployment policy，不执行它们。因此：

- 不决定 typed JSON 是否支持 server stream；
- 不决定 Router 与 Host 谁计算或执行 timeout；
- 不新增 stream framing、content type、terminal/error表示；
- 不把任何临时选择写入 shared model。

如果实现发现 linked checkpoint 本身必须先决定这些语义，立即停止并报告
`TASK_NOT_EXECUTABLE`，不得自行选择。

## 写入范围

主要 owner：

- `artifact-model/src/runtime_assembly.rs`、schema/lexical exports；
- `artifact-identity/src/runtime_assembly*`与直接 assembly identity tests；
- `deployment/src/assembly/**`及其直接 tests/fixtures；
- `runtime/loader/src/runtime_assembly/**`及其直接 tests；
- `runtime/linker/src/assembly/**`及其直接 tests；
- 只为上述 strict v2 编译所需的同 crate exports/error variants。

允许删除只服务旧 operation ingress 的 assembly error、helper和fixture。直接触碰的超长或重复
gateway linking逻辑必须抽成职责明确模块，不能继续膨胀已有数百行文件。

禁止修改：

- F351/F355 gateway/deployment DTO、normalization、identity或generation；
- F357 compiler projection与HTTP authoring；
- ServiceContract、ServiceProtocol、PackageArtifact generation；
- runtime Host/request/transport/eval/activation；
- Router、test-runner、cross-system request wire；
- WebSocket业务模型；
- stable/live配置、lockfile或三仓库service源码。

若实现需要改变上述公共 owner、新增runtime codec DTO、复制compiler schema逻辑，或必须同时迁移
Host/Router/test-runner，按“有界探查与强制停止”处理。

## 完成标准与验证

先枚举并确认 selector非零，再运行聚焦命令：

```bash
cargo test -p skiff-artifact-model runtime_assembly -- --list
cargo test -p skiff-artifact-identity runtime_assembly -- --list
cargo test -p skiff-deployment assembly -- --list
cargo test -p skiff-runtime-loader runtime_assembly -- --list
cargo test -p skiff-runtime-linker assembly -- --list

cargo test -p skiff-artifact-model runtime_assembly
cargo test -p skiff-artifact-identity runtime_assembly
cargo test -p skiff-deployment assembly
cargo test -p skiff-runtime-loader runtime_assembly
cargo test -p skiff-runtime-linker assembly
cargo check -p skiff-artifact-model -p skiff-artifact-identity -p skiff-deployment \
  -p skiff-runtime-loader -p skiff-runtime-linker
rustfmt --edition 2021 --check <changed-rust-files>
git diff --check
```

若已有测试名使某个 filter得到零测试，先补或改用能证明该 owner 的非零精确 filter，并在 result
记录实际枚举结果；不得用零测试当作 PASS。

正向至少覆盖：

- zero gateway/zero ingress；
- one entry/one selector；
- one entry/multiple selectors且共享同一 linked entry；
- multiple deployments的全局 canonical selector map；
- private handler以及 optional pre/guard 的 exact target与signature；
- internal service operation linkage在同一 fixture中保持可用；
- assembly identity mutation与reorder稳定性。

负向至少覆盖：

- v1 schema/prefix、旧 `globalIngress`、旧 contract/operation fields、missing field；
- duplicate selector、dangling deployment、missing key、wrong identity；
- assembly/deployment ingress缺失或多出；
- missing/wrong-package callable、nested ID/target错配；
- missing/ambiguous callable signature；
- external ingress通过ServiceContract descriptor或`ContractOperationId`恢复的残留反搜。

Production反搜至少证明新 external assembly binding/link类型中没有
`ContractOperationId`、`ServiceContractRef`或operation descriptor lookup；internal service-call
owner中的合法使用不计为残留。

不运行workspace/root、stable/live，不push。

## 风险、证据与验收

- 风险：高（canonical RuntimeAssembly schema/identity与runtime admission前置链接）。
- 开发Agent拥有上述聚焦测试与静态检查证据。
- 合流后主Agent只运行一次便宜 combined probe，覆盖 identity、assembly、loader、linker共同接线。
- 独立验收与完整 gate等所有 C3 consumers 合流形成稳定候选后统一执行；本 leaf 不单独运行昂贵
  workspace/root gate。
- 任何 schema/identity/link类型改动都会使本 leaf证据失效；纯 result文档与 bit-identical merge
  不失效。

## 有界探查与强制停止

开发Agent可为阻止正确实现的一个具体未知量派至多一个只读子Agent，例如确认
“implementation local ABI 中 callable ID 到唯一 signature 的 canonical lookup”。必须写明读取
范围、返回证据和停止条件；该子Agent不得再派下一层。

从Agent启动到第一次实际代码修改默认不超过5分钟。若有界探查证明：

- 需要新的公共 codec/type-plan模型；
- 任务实际拆成独立 RuntimeAssembly、loader和linker owner且无法形成一个共享checkpoint；
- 必须先改 request wire、Host、Router或test-runner；
- typed stream/timeout/WebSocket仍有多项会改变本任务实现方向的未知量；

则当前任务立即结束：

- `TASK_SCOPE_EXPANDED`：列出被证伪前提、精确路径、实际owner、有效提交与最小拆分；
- `TASK_NOT_EXECUTABLE`：列出缺失公共决策及最小前置。

不得静默扩大写入范围、继续派Agent或把只读探查写成Completed。

## Worktree与交付

- worktree：`/Users/geek/workspace/skiff-p5-f358-runtime-assembly-gateway`
- branch：`codex/p5-f358-runtime-assembly-gateway`
- 从包含本task的integration checkpoint创建；
- production/tests一个commit，result一个commit；
- result写入同目录
  `P5-F358-runtime-assembly-http-gateway-linking-result.md`；
- result记录exact base/production commit/tree、schema/identity、assembly union、loader join、
  linked candidate API、自验收矩阵、非零selector、反向搜索与明确下游残余；
- worktree保持clean，不merge/rebase integration，不push。
