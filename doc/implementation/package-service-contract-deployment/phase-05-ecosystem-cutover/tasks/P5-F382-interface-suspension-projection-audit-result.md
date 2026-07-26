# P5-F382 Interface suspension projection audit result

状态：Complete（只读审计；需要语言语义决策）。

## 结论

`TASK_NOT_EXECUTABLE`

现有规则已经决定：

- suspension/effect 是 method-level contract，不是 package/service/file 级开关；
- interface 的完整 method signature、conformance、Package Local ABI 和被 service operation 引用时的
  service protocol identity 必须包含 effect contract；
- `maySuspend` 只表示方法可能到达真实等待；runtime 只在实际等待时释放 actor 执行权，不存在显式
  `yield`。

但现有语言没有 interface suspension 声明语法，AST、source requirement fact 和 FileIR interface
operation 也没有对应字段；同时规则没有定义 `maySuspend=true` requirement 与
`maySuspend=false` implementation 是合法 subeffect，还是 exact mismatch。当前真实 production
population 同时有 49 个 suspending implementation 和 47 个 non-suspending implementation，且多个同一
interface operation 的实现分属两类。因此无法由 Relay 单点失败唯一推出 source spelling、默认值和
conformance variance，也不能安全地直接修改 compiler。

唯一后继是用户在本文“需要选择的语义”中选择一项；选择前没有 production/test implementation
boundary。

## 审计锚点与边界

- Skiff 审计 worktree：`21e00f9c91f844eace013c434f7b4b7692017e74`；production code tree
  锚定其父提交 `b240c380dd1535d6405d39337694f53e4d385ce7`。
- Skiff packages worktree：`3653a294cfb92e60e220dcccc94bc8e8add65b33`，tree
  `93602c0e99ef15cf539334931e522d5ba844871c`。
- Internals/Relay clean worktree：
  `68c7d679899bf942060fe407270cea60b7ba85ca`，tree
  `de19e938259e6f023cd206791f4bfb9c5e4d03d9`。
- production population 是父节点真实生态依赖闭包中、下表所列 package/service 的全部 production
  `.skiff` source；
  `*.test.skiff` 按 `doc/reference/static-semantics.md:202-207` 排除。
  `*_test_support.skiff` 没有该后缀，仍属于 production source set，故纳入主统计。
- Skiff 和 Internals production 均未修改；没有操作 stable instance、live store 或外部上游。
  所有编译、FileIR、artifact 和 identity 探针只写入 `/tmp` 隔离副本和隔离 store。

## 对四个精确问题的直接回答

### 1. source interface 是否已有 suspension/effect 声明

没有。

- `doc/reference/syntax.md:17` 的关键字集合没有 `async`、`suspend`、`effect` 或 `yield`；
  `:60-64` 的 function header 只有 `native` / `static`、名称、类型参数、参数和返回类型。
- `doc/reference/interface.md:59-64` 只规定普通 function signature 和 `self: Self`。
- `syntax/src/ast.rs:311-325` 的 `InterfaceOperation` 没有 effect/suspension 字段。
- `syntax/src/parser.rs:628-742,1495-1537` 只解析已有 function modifier，并禁止 requirement body、
  unsupported method-level generics 和不允许的 modifier；没有可写入 suspension contract 的分支。
- 对 production source 的 token 搜索没有发现可用的 suspension declaration；相关单词只出现在注释。

所以 Relay 不是“漏写了一个已有声明”。当前 source 根本无法表达该 requirement。

### 2. compiler 是否漏投影了已有声明

不是已有声明的投影遗漏，而是 requirement fact 从 source 开始就缺字段，随后又被 projection 人工补成
`false`：

- implementation effect 能正确从 body/call graph 推导为 `true`；
- interface requirement 没有 `may_suspend`；
- source conformance 没有比较 suspension；
- FileIR interface operation 没有 suspension 字段；
- Package Local ABI 的 interface method DTO 虽有字段，但两个生产 projection 都硬编码为 `false`。

这形成 split-brain：implementation/public callable 拿到真实 inferred effect，interface side 永远为
`false`，直到 public-instance identity validator 才报错。

### 3. canonical owner

按现有文档，canonical owner 应是 **interface method 的显式 ABI/effect requirement**：

- `doc/reference/interface.md:86-89` 要求 conformance 比较完整 canonical signature，明确包含
  effect contract；
- `:112-114` 规定 conformance 是 Package API / Local ABI fact，service operation 引用时还进入
  service protocol identity；
- `:242-258` 要求 artifact 保存 method requirement signature，并用于静态解析、ABI identity 和
  compatibility；
- `doc/architecture/package-service-contract-deployment.md:675-682` 要求 `PackageSourceModel`
  分别拥有 exact executable signature 和 exact interface requirement facts，source conformance 比较二者，
  lowering/projection 不得从 FileIR 重算。

package/service/file 级 effect 会把同一 owner 中互不相关的方法合并，无法表示本次扫描中的 method-level
混合行为，且与上述 exact method facts 不符。绑定 concrete implementation 后才决定、或完全从
interface conformance/identity 删除 `maySuspend`，都需要显式修改上述语言/架构规则，不能作为当前规则的
机械修复。

现有文档仍没有决定 interface requirement 是 exact effect 还是“允许更少 effect”的 upper bound；
这正是阻塞点。

### 4. 多实现、调用方、actor 与 ABI 当前看到什么

当前行为并不是完整稳定语义：

- concrete/local implementation 的 inferred `may_suspend` 是真实 executable fact；
- `any I` receiver call 在
  `compiler/source/src/resolved_call_targets/builder.rs:352-354` 被记为
  `Unknown(UnsupportedDynamicDispatch)`，suspend analysis 在
  `compiler/lowering/src/suspend_analysis.rs:628-695` 对 Unknown/contract/dependency call
  保守返回 `true`；
- local interface method table 的
  `artifact-model/src/executable.rs:668-693` 只保存 params/return，不保存 effect；
- public-instance boundary 使用 concrete public callable 的实际 bit；
- runtime 按 `doc/architecture/actor-model.md:77-84,109-111` 只在 stream next、异步 service call、
  timer 等实际等待时释放 actor；compiler 仍必须把可能等待的方法标记为 `maySuspend`。没有 suspension
  point 的同步段不交替，也没有显式 `yield`。

因此当前 `any I` caller 总是保守地看到“可能挂起”，public service caller 却看到 concrete binding 的
actual bit；interface ABI 本身没有提供统一稳定承诺。多个实现的稳定语义必须由下文 exact 或 upper-bound
选择补全。

## Source 到 contract 的逐跳追踪

| 阶段 | 当前 canonical fact | 生产位置 | 结果 |
| --- | --- | --- | --- |
| Source executable | `SourceExecutableSignature.may_suspend` | `compiler/source/src/contract_type_resolution.rs:46-83`；`compiler/source/src/contract_type_resolution/executables.rs:18-40,198-210` | 从 exact callable effect facts 正确生成，并进入 `PackageCallableSignature`。 |
| Source interface requirement | params/return/receiver/type params/flags，无 suspension | `compiler/source/src/contract_type_resolution.rs:94-115` | requirement 无法拥有 effect fact。 |
| Source conformance | 解析 exact requirement 与 exact executable，但只比较 receiver、params、return | `compiler/source/src/contract_type_resolution/interfaces/conformance.rs:148-224,261-395` | suspending implementation 当前会通过 source conformance；这不是一种成功 source spelling。 |
| FileIR implementation | executable signature 含 inferred `may_suspend` | `artifact-model/src/executable.rs:18-55`；`compiler/lowering/src/suspend_analysis.rs` | Relay 两个真实 method 均为 `true`。 |
| FileIR interface | `InterfaceOperationIr` 无 suspension 字段 | `artifact-model/src/types.rs:234-260`；`compiler/lowering/src/interface_declaration_lowering.rs:19-126` | Relay requirement 无法携带该 fact。 |
| Package interface method | `InterfaceMethodSignature` 有 `may_suspend` | `artifact-model/src/package_unit.rs:203-219` | DTO 有承载位置。 |
| Interface/public-instance projection | interface method bit 硬编码 `false` | `compiler/core/src/package_interface_methods.rs:168-210`；`compiler/projection/src/package_artifact/callables/mod.rs:286-390` | 两条 projection path 重复产生错误 canonical value。 |
| Public callable projection | 使用 source exact executable signature | `compiler/compiled/src/package_callable_signatures.rs:61-88`；`compiler/projection/src/package_artifact/callables/signatures.rs:104-162` | Relay 两个 public callable 均为 `true`；`PackageCallableId` 仍由稳定 public path 决定。 |
| PackageArtifact validation | interface method、implementation link、public signature 三方 exact equality | `artifact-identity/src/package_artifact/validation/public_instances.rs:435-597` | 在 `:582-594` 报 `return or suspension semantics disagree with its interface`。 |
| Boundary projection | actual public callable bit 进入 `BoundaryOperationContract` | `compiler/projection/src/package_artifact/boundary/types.rs:17-78` | `true -> cooperative`，`false -> notCancellable`，并写入 `maySuspend`。 |
| ServiceContract | available boundary projection 成为 operation descriptor | `compiler/contract/src/projection.rs:61-142`；`compiler/contract/src/compile.rs:29-78` | operation stable ID 先生成，完整 descriptor 再进入 protocol identity。 |

真实 Relay 的两条 operation 都呈现同一断裂：

| operation | interface projection | implementation/public callable | production 结果 |
| --- | ---: | ---: | --- |
| `relayProxy.responsesCompleted` | `false` | `true` | validator 拒绝 |
| `relayProxy.responsesCompletedResult` | `false` | `true` | 首条失败修复后也会拒绝 |

## 全量 production interface 审计

“pair”指一条 interface method requirement 与一个声明 `implements` 的 concrete method 配对。
由于当前 interface projection 恒为 `false`，下表的“一致”即 implementation inferred
`maySuspend=false`，“不一致”即 inferred `true`。

| package/service | pairs | 一致：impl `false` | 不一致：impl `true` | 证据 |
| --- | ---: | ---: | ---: | --- |
| `skiff.run/std` | 0 | 0 | 0 | exact production compile |
| `skiff.run/http-session` | 0 | 0 | 0 | exact production compile |
| `skiff.run/track` | 0 | 0 | 0 | exact production compile |
| `agine.ai/llm-api` | 0 | 0 | 0 | 声明 `LlmClient`，本 package 无 implementor；exact compile |
| `agine.ai/llm-providers` | 0 | 0 | 0 | exact production compile |
| `agine.ai/agent` | 73 | 39 | 34 | exact emitted FileIR；临时副本只修正一个与 effect 无关的 ambiguous nullable literal type |
| `agine.ai/codex-relay` | 2 | 0 | 2 | exact emitted FileIR；真实 method body 未改，临时关闭 API graph 以读取 pre-validator FileIR |
| `agine.ai/aihub` | 5 | 3 | 2 | exact emitted FileIR；临时副本只规范 receiver/无关 F383 literal typing，并关闭 legacy API graph |
| `skiff.run/account` | 1 | 1 | 0 | exact emitted FileIR；临时副本只规范 interface receiver 并关闭 legacy service graph |
| `agine.ai/api`（Agine） | 15 | 4 | 11 | 按 exact production suspend-analysis 规则对真实 method body/call graph 分类；见下述限制 |
| **合计** | **96** | **47** | **49** | Relay 只占 2/49 |

Agine 当前锚点另有排队中的 F383 expression-type 与 legacy `ErrorPayload` blocker，不能生成完整 exact
artifact；因此其 15 条是同一 exact suspend-analysis 规则对 production method body 和已解析调用类别的
逐条分类，不伪装成 emitted FileIR。其余有 pair 的 package 均以 exact compiler 的 emitted FileIR
核对。

若人为排除文件名含 `_test_support.skiff`、但并非 `*.test.skiff` 的 production 文件，补充统计为
50 pairs：14 个 `false`、36 个 `true`。主结论不变。

没有任何“suspending interface implementation 成功声明 effect”的 source 写法。34 个 Agent
suspending pair 能产出 artifact，是因为 source conformance 根本不比较 suspension；Relay 两条进入
public-instance validator 后即失败。

### 同一 operation 的混合实现

扫描中至少以下 requirement 同时存在 `false` 与 `true` implementation：

| interface operation | impl `false` | impl `true` |
| --- | ---: | ---: |
| `LlmClient.webSearch` | 3 | 1 |
| `AgentEventReceiver.receive` | 3 | 1 |
| `SubagentDelegate.configure` | 3 | 4 |
| `SubagentDelegate.deliver` | 2 | 5 |
| `SubagentDelegate.cleanup` | 6 | 1 |
| `ToolProvider.providerName` | 5 | 1 |
| `ToolProvider.tools` | 4 | 2 |
| `ToolProvider.execute` | 4 | 2 |
| `DrainCheckpointStore` methods | 1 each | 1 each |
| `DrainToolPort.execute` | 1 | 1 |
| `PendingUserMessageProbe` method | 1 | 1 |

所以“所有 interface method 默认为/写成 false”会继续拒绝 49 个 pair；“只要某实现会挂起就把共享
requirement 写成 true”若采用 exact equality，又会拒绝同一 requirement 下的 non-suspending
implementations。upper-bound 与 exact 不是可由数据替用户决定的等价实现细节。

## Identity 与可观察 contract 影响

生产代码的 identity preimage 已经决定：

- `artifact-identity/src/package_artifact/projection.rs:24-50` 把完整 `public_symbols` 写入 Package
  Local ABI preimage；interface/public callable suspension contract 改变会改变 Local ABI identity。
- `artifact-identity/src/package_artifact/projection.rs:53-169` 的 build preimage 包含 Local ABI identity、
  implementation symbols、FileIR 和 boundary projections；相关变更会改变 Package Build ID。
- `artifact-identity/src/contract.rs:165-184` 的 `ContractOperationId` 只由 service identity 和 stable
  operation key 决定，不含 operation descriptor；只改变 suspension contract 时 operation ID 稳定。
- `artifact-identity/src/contract.rs:186-207` 的 `ServiceProtocolIdentity` 包含完整 operation descriptor；
  `maySuspend`/cancellation 改变会改变 protocol identity。
- `PackageCallableId` 由 package/public path 构造，也保持稳定。

隔离探针用同一 Relay public API 比较 non-suspending `{}` method 与真实 suspending body。真实 body
只能通过一个仅跳过两条 suspension equality validator 的诊断 compiler 副本完成 projection；该副本
没有改变 projection 或 identity 代码，不能视为 production acceptance。

| fact | non-suspending probe | real suspending projection |
| --- | --- | --- |
| Package Local ABI | `skiff-package-local-abi-v6:sha256:f0eb79a034d6aaf4fdb722734e1c1c44b14cb884ec6c03b030481a1034ca46d3` | `skiff-package-local-abi-v6:sha256:ba05fa39b4e519223dabe0aa3881d7521f180dfe847ae200ed3a32cb2afa58e8` |
| Package Build | `skiff-package-build-v8:sha256:577c58d3eae9d9853362779a827e6f1280205e7261cd84fc17b23cc420fd610e` | `skiff-package-build-v8:sha256:45832a04dec79be7e7713338f505741fcdbbd3c0969a557117050c6a3d5ec636` |
| Service protocol | `skiff-service-protocol-v4:sha256:39234f150af71250c8184925f968a2183f10c11bf2d6dc3866815dab38834da5` | `skiff-service-protocol-v4:sha256:59f4faeb5dcf8cbbbfad4cf567776bb6aed435072181dc22a9a086762881d913` |
| operation contract | `maySuspend=false`, `notCancellable` | `maySuspend=true`, `cooperative` |
| `responsesCompleted` operation ID | `skiff-contract-operation-v1:sha256:b62d89d553cc0607b2627b047d2a5ab4665c70f05f900babbce249def47099ef` | 相同 |
| `responsesCompletedResult` operation ID | `skiff-contract-operation-v1:sha256:51fa082dd0d33b09f45e4900805c28801cb3108b4eac813697e66e5f8a6b007d` | 相同 |
| Package callable IDs | 两个稳定 `pkg-callable:agine.ai/codex-relay:relayProxy.*` path | 相同 |

Package Build 两侧还包含不同 body，故其具体 hash 不是 effect-only 对照；但 build preimage 对 Local ABI
和 implementation facts 的结构性依赖已经证明 effect 修复必然重建 build。Local ABI 和
ServiceProtocol 对照则直接显示 caller-visible contract 差异。

ordinary `self: Self` interface 本身不满足当前 PackageSchema boundary closure；Relay 的 schema record
是 receiver record，不是 bare interface。因此本例不会仅因补 interface effect 而产生新的
PackageSchema type ID。若未来 callback/schema-eligible interface 使用同一字段，必须另补 schema
projection 测试，不能从本例外推。

## 需要用户选择的语义

以下方案在 caller、actor、ABI 和迁移范围上可观察地不同；现有规则不能在它们之间唯一选择。

### A. Interface effect 是 upper bound

定义 method-level source declaration 和默认值；`requirement=false` 只接受 non-suspending
implementation，`requirement=true` 接受 `true` 或 `false` implementation。

- `any I`、public instance 和 service caller 看 interface 声明的稳定上界；
- actor 仍只在 concrete execution 实际等待时释放执行权；
- 当前 mixed operations 可继续共享一个 interface，只需把“至少一个实现可能挂起”的 requirement
  声明为 `true`；
- interface owner/implementor 的 Local ABI/build 需要重建；被 service operation 使用时 protocol identity
  改变，operation/callable ID 稳定；
- 用户仍需选择 source spelling，以及未声明时默认 `false` 还是强制显式声明。

### B. Interface effect 是 exact contract

定义 method-level source declaration 和默认值，并要求 inferred implementation bit 与 requirement
exact 相等。

- caller 获得 exact ABI bit；
- 当前 mixed operations 不能继续共享同一 requirement：必须拆 interface/operation，或另行设计
  “将 non-suspending implementation 显式提升为 may-suspend contract”的语义；
- 未额外设计提升规则时，共享 requirement 标为 `true` 会拒绝该 requirement 下所有 `false`
  implementation；
- identity 影响与 A 相同，但 source migration 更广；
- 用户仍需选择 source spelling/default，并决定是否另开 effect widening 设计。

### C. Effect 由 concrete public-instance binding 投影

维持 interface requirement 无 suspension；删除 validator 的三方 suspension equality，public callable 和
ServiceContract 使用 concrete implementation 的 inferred bit。

- 同一 interface 的不同 public instance 可发布不同 `maySuspend`、cancellation 和 service protocol
  identity；
- local `any I` 没有 interface bit，继续保守视为 `true`；
- interface owner 的 Local ABI 不因 effect 改变；public implementation 的 callable ABI/build 和
  service protocol 仍随 actual bit 改变；
- 对 Relay 是最窄 compiler 修复，但必须同步改写 `interface.md` 和架构中“effect 属于完整 exact
  requirement/conformance”的既有规则。

### D. `maySuspend` 不参与 interface 或 public/service identity

从 interface、public callable ABI、boundary/service contract identity 中移除该 bit，并为所有 caller
采用固定保守 suspension/cancellation policy。

- implementation 仍可保留 runtime/internal scheduling 分析；
- effect 改变不再改变 Local ABI 或 ServiceProtocol；
- caller 不能获得 `notCancellable`/non-suspending 的稳定保证，现有
  `BoundaryOperationContract.maySuspend` 与 cancellation 映射必须重定义；
- 这是最大范围的文档、artifact、contract、runtime/linker 语义变更，不是 Relay 局部修复。

package/service/file 级 effect 不单列为最小方案：它无法保留现有 method-level exact signature，也无法
表达本次已观测的同 package、同 interface 内混合行为；若用户要选择它，必须先改写 interface/effect
架构，而不是作为上述方案的实现细节。

## 决策后的重建与验证 DAG

A/B 的全生态最小顺序是先改 spec/compiler，再 fresh 重建所有 interface owner/implementor 及其依赖：

```text
std
├─ http-session ──> track ───────────────────────────────┐
│  └─ Account                                            │
└─ llm-api ──> llm-providers ──> Relay ──> Aihub ───────┤
   └─ Agent ─────────────────────────────────────────────┤
                                                        └─ Agine
```

其中 Relay 首个独立 revalidation DAG 是
`std -> llm-api -> llm-providers -> Relay`；若 Relay protocol identity 改变，Aihub、Agine 必须使用新
receipt 重建，不能复用旧锁定。

C 至少重建 compiler、所有 public-instance producer（Relay、Aihub、Account）及其 service consumers；
Relay 的最小 fresh dependency DAG 仍是上述四节点。Agent 的 internal conformance 不因该方案本身改变，
但 Agine 若消费新 Aihub protocol 仍需重建。D 要重建全部 package artifacts、service contracts、
assemblies/deployments 和 consumers。

无论选择 A/B/C，production/test 后继至少必须覆盖：

1. parser/AST/source exact requirement fact 和默认值；
2. `false/false`、`true/true`、`true requirement/false implementation`、`false requirement/true
   implementation` 四格 conformance；
3. interface/implementation FileIR、Package Local ABI/public callable projection和 validator；
4. local `any I` method table/caller suspend analysis；
5. boundary `maySuspend`/cancellation、Local ABI/build/protocol identity 与稳定 operation/callable IDs；
6. 真实 Relay 两条 operation 的 fresh artifact/contract receipt，而不是 `{}` 或 validator-waiver
   probe；
7. Aihub/Agine service consumer 以新 protocol receipt fail-closed/rebuild。

在用户选择 source declaration/default/variance 或明确改写现有 ABI 规则前，不能把任何一组测试期望
冻结为 canonical。
