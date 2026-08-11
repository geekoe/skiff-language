# Runtime 错误分类与 Skiff 异常投影 — 实施计划

状态：Planned；架构契约已冻结（2026-08-11）。

权威目标模型：
[`doc/architecture/runtime-error-to-skiff.md`](../architecture/runtime-error-to-skiff.md)。

本文只负责从当前多 crate 手写 `RuntimeErrorPayload` / `catch_projection` 实现迁移到目标模型。
若本文与架构文档冲突，以架构文档为准；实现过程中不得在本计划里重新决定用户语义。

## 1. Goals

本计划完成以下目标：

1. 保留专门 Rust error type，只把纯诊断信息统一为 `code + message + diagnostic frames /
   correlation`，不引入全局穷举 `RuntimeFaultData`。
2. 把 diagnostic formatting 与 Skiff projection 拆成独立能力。
3. 建立 compiler-owned platform error projection catalog，以 canonical Skiff type IR 为唯一字段 schema，
   生成 Rust typed payload、catch identity、materializer 和 service codec。
4. 让 actual projection 显式依赖 failure site 与 operation admission；default deny。
5. 保留 `RecoverableBoundaryError`，解决 large error，并为未来第一个明确准入的 production operation
   预留生成式 `std.recoverable.BoundaryError`，不提前发布无 owner 的 public type。
6. 阻止 generic JSON/BSON/driver source error 因底层 Rust 类型相同而自动暴露给 Skiff。
7. 保持 user exception、fixed service failure、opaque forwarding、internal terminal 和
   cross-service correlation 的现有 canonical 语义。

## 2. Non-goals

- 不增加 checked exception、operation throw set 或 ServiceContract error list。
- 不新增 universal Skiff `RuntimeError { code, message, details }`。
- 不让所有 production recoverable call site 在本计划中自动变得可捕获。
- 不把 recoverable internal code 发布成可供业务分支的 public string protocol。
- 不把完整 Rust backtrace、provider stack、expected plan 或 raw source error 放进 public payload。
- 不以兼容层保留旧手写 projection table；迁移完成后直接删除旧 owner。
- 不把 DB、dispatch、queue、materialization 的业务重试策略合并成统一 retry policy。

## 3. Current baseline and gaps

### 3.1 Shared diagnostic DTO

`runtime/request-contract/src/error.rs` 当前定义：

```rust
RuntimeErrorPayload { code, message, status, details }
WirePayload::payload()
WirePayload::catch_projection()
```

问题：

- diagnostic payload 与 catch projection 被放在同一 trait，容易误把相同 string code 当作相同
  catch identity；
- `details: Json` 同时承载 diagnostic frame、typed public payload候选和开放内部 detail；
- cancellation/scope terminal 必须绕开这个 total-looking API；
- 各 crate 重复构造相同字段 JSON。

### 3.2 Repeated Rust error conversion

`runtime/boundary`、`runtime/native`、`runtime/eval` 和 `runtime/host` 各自拥有
`RuntimeError`。其中 decode、bytes、DB、file、HTTP、recoverable 等 variants 和
`From` / clone mapping 存在重复。

并非所有重复都应合并：上层真正改变 terminal、retry、ownership 或 wire 语义时仍需显式
reclassify。语义未变的下层 error 应逐步改为 wrapper/delegation。

### 3.3 Manual platform registry and payloads

当前 `PlatformBuiltinErrorIdentity`、`from_symbol()`、`symbol()`、各 crate
`ordinary_catch_projection()`、eval materialization 和 service channel codec 由多处手写。

风险：

- `.skiff` 字段变化与 Rust JSON payload 静默漂移；
- 同一 symbol 在 compiler/runtime 形成不同 registry；
- 新增 error 需要跨多个 match 手动同步；
- raw/internal message 容易被误放进 public payload。

同一问题也存在于 bytecode 路径：artifact opcode failure contract 当前可以保存手写 catch symbol，
collection access 等 VM producer 因而可能绕过 runtime error registry。迁移 inventory 必须覆盖
artifact-model、compiler lowering/emission、bytecode verifier、linked bytecode、loader 和 VM，不能只审计
Rust boundary/eval enum。

当前 service `PlatformError` wire 使用 closed Rust enum；未知 platform identity 在 outer envelope decode
时直接失败，尚不能按 canonical opaque cause 转发。Catalog 演进必须同时处理 artifact/runtime fingerprint
与跨版本 service envelope，不能只生成本地 materializer。

### 3.4 Recoverable error

`RecoverableBoundaryError` 内嵌 context、expected plan 和 detail，放大多个上层
`RuntimeError`，触发 `result_large_err`。当前它有 diagnostic payload但没有 catch projection。

目标不是直接给它增加全局 `catch_projection()`；目标是：

- leaf error 内部装箱；
- 只有具体 operation semantic adapter 才能把 projectable value rejection 转成 generated DTO；
- artifact/state/sealed-payload 等 integrity failure 即使共用这个 Rust type 也不能随 operation 一起准入；
- 在首个 production owner 出现前不增加 public std surface。

### 3.5 Context-insensitive source conversion

当前部分 decode mapping 依赖 `DecodeTarget` string 或在多个 crate 中复制 target-to-symbol
逻辑。需要收敛到创建 semantic wrapper 的 source operation；generic `serde_json::Error`、
BSON 和 backend driver error 本身不再携带 public identity。

## 4. Implementation principles

### 4.1 Three destinations for error information

实现时逐字段判断：

| 字段用途 | Target |
| --- | --- |
| Rust/Skiff 程序需要据此决策 | 专门 Rust type；若公开则进入 exact generated Skiff payload |
| 受限 telemetry 需要查询/聚合 | bounded diagnostic attribute；不进入 public schema |
| 只供人排查 | message 或 Rust source chain |

不得为了“以后也许有用”把所有 detail 提升到共享 enum。若未来代码需要处理某个 detail，再新增专门
typed error / field 和对应 contract。

### 4.2 Projection has two gates

任何 Rust-to-Skiff mapping 都经过：

1. **Capability gate**：semantic adapter 构造 catalog/codegen 生成的 typed DTO，证明它对应 exact
   Skiff type；底层 Rust error 本身不能自报 identity。
2. **Admission gate**：runtime-owned continuation guard 与 closed
   `(operation, projection key, semantic class, phase)` policy 证明本次 failure 可以交给业务 catch。

只有 capability 没有 admission 时，仍按 request/control/background failure 处理。

Admission 不证明 operation 没有副作用。可捕获错误可以表示 remote effect 已发生、未发生或 outcome
unknown；projection 只需与 operation reference 中的 effect certainty 一致。业务 catch 后是否重试继续由
effect、幂等和补偿 contract 决定。

### 4.3 No string inference

迁移期间和完成后都禁止新增以下逻辑：

```text
payload.code == "std.foo.Error" -> catch identity
Rust type name -> Skiff symbol
JSON object shape -> public error type
Display message prefix -> retry/catch class
```

## 5. Milestones

### M0 — Inventory and behavior freeze

状态：Pending。

先建立当前 error owner 和 observable behavior 矩阵，不修改 production 语义。

工作：

1. 枚举以下路径的 error type、diagnostic mapping、catch projection、request terminal 和 service
   export：
   - `artifact-model` opcode failure contracts；
   - compiler lowering / emission 的 failure projection；
   - `runtime/request-contract`；
   - `runtime/model`；
   - `runtime/boundary`；
   - `runtime/native`；
   - `runtime/capability-context`；
   - `runtime/service-db`；
   - `runtime/eval`；
   - `runtime/request`；
   - `runtime/host`；
   - `runtime/transport`；
   - `runtime/linked-bytecode`、`runtime/bytecode-verifier`、`runtime/loader` 和 `runtime/vm`。
2. 为每个 production source operation 标注：
   - internal terminal；
   - diagnostic-only；
   - existing projectable platform error；
   - user/fixed service error；
   - control/ingress/background owner。
3. 为所有 recoverable codec 调用点记录 boundary kind、failure phase、effect commit point、active
   Skiff call site、semantic failure owner和当前 canonical admission。
4. 枚举 opcode/VM 中的 hand-written catch symbol、payload field builder 与 invariant-terminal，特别是
   collection access failure。
5. 记录 service `PlatformError` 对 known-valid、known-malformed 和 unknown identity/payload 的当前行为。
6. 把当前正负行为写成测试，不把临时审计表新增到 architecture 文档。

完成标准：

- 所有 `ordinary_catch_projection()` producer 都有明确 owner。
- 所有 artifact/opcode/VM catch producer 和手写 symbol 都有明确 owner。
- 所有 `RuntimeErrorPayload.code` 与 catch identity 不一致的有意案例有负测试。
- cancellation、scope terminal、fixed service failure、collection failure 和 generic DB/JSON decode 的
  现状被测试冻结。
- 没有 production 改动。

### M1 — Box recoverable leaf error

状态：Pending。

这是独立、纯 Rust representation 改动，不等待 public projection。

工作：

1. 把 `RecoverableBoundaryError` 的大型 context/expected/detail 收进一个 boxed inner data。
2. 保持 constructor、accessor、`Display`、`Error` source、code 和 diagnostic 输出不变。
3. boundary/native/eval/host 继续按值移动同一个小 wrapper；不在每层分别增加 `Box` variant。
4. 删除因 large error 引入的局部 allow；不增加新的 `clippy::result_large_err` allow。

完成标准：

- boundary 与所有嵌入它的上层 error type 不再触发 `result_large_err`。
- recoverable unit tests和现有 request/host error tests行为不变。
- 本里程碑不新增 catch identity、std symbol 或 service envelope变化。

### M2 — Separate diagnostic capability from projectability

状态：Pending。

工作：

1. 在 `runtime/request-contract` 定义最小 diagnostic capability：
   - typed/constant diagnostic code；
   - human message；
   - 可选受限 diagnostic attributes；
   - 不包含 catch identity。
2. 定义独立且 sealed 的 projection capability seam；M2 只隔离现有手写 bootstrap adapter，M3 开始后
   exact projection key 和 typed public payload只能由 catalog/codegen 创建。
3. 保留 transitional `WirePayload` adapter供未迁移 consumer 使用；新 production path不得直接实现
   新的手写 `catch_projection()`。
4. eval/host 的 source/diagnostic frame继续由 wrapper逐层追加，不要求 leaf error 自带 stack。
5. `Cancelled`、scope terminal、fixed service carrier 继续使用专门 control/carrier path，不被强制实现
   ordinary diagnostic/projectable trait。

完成标准：

- code equality不能授予 catch identity。
- Diagnostic-only error不能调用 generated materializer。
- Projectable error payload不读取开放 `RuntimeErrorPayload.details`。
- existing behavior通过 transitional adapter保持，尚未迁移的 crate可分阶段编译。

### M3 — Canonical platform error projection catalog and codegen

状态：Pending。

M3 的第一个提交是原子 canonical cutover contract，不修改 production code。它必须同时更新：

- `doc/architecture/bytecode-vm.md` §3.1：增加第六个
  `PlatformErrorProjectionRegistryRef` authority，固定其 bytecode/PackageArtifact carrier、identity preimage
  与 v7/generation-v5/PackageArtifact-v15/Package-build-v14 hard cut；
- `doc/architecture/package-service-contract-deployment.md` §6.3：把 closed
  `builtinErrorIdentity` 改为 `{ projectionKey, entryFingerprint, encodedPayload, traceId, errorId }`，固定
  token/fingerprint/64-KiB payload bounds、unknown opaque与known malformed规则；
- runtime session/router contract：`skiff-runtime-frame-v5` 的 `runtime.capabilities` 携带 exact registry
  descriptor，Router route admission与PackageArtifact descriptor exact-match。

这三项与本文 §5.3 必须在同一提交一致。未完成这个 contract commit 时，不得开始 catalog/schema/code
改动；不能留到 M8 补文档。

新增一份 machine-readable、compiler-owned catalog，建议路径：

```text
std/error-projections.yml
```

Catalog entry 至少包含：

- stable projection key；
- exact public symbol；
- producer family / semantic adapter owner；
- public message policy；
- service envelope kind；
- fallback policy。

Catalog 不包含 public field list、field type 或 field order。`.skiff` type declaration 和
`std/api.yml` 是 public symbol/schema surface 的唯一事实源；generator 必须通过 compiler 解析 exact
symbol 取得 canonical type IR，catalog 只把 projection policy关联到该 IR。

仓库固定使用一个 generator 和 checked-in generated source，不使用各 crate 的 Cargo `OUT_DIR` generator。
Generator 一次生成：

- Rust `PlatformErrorProjectionKey` / exact identity registry；
- 每个 entry 的 typed projection payload；
- payload encode/decode 与 field materializer；
- compiler/linker catch identity descriptor；
- service `PlatformError` codec dispatch；
- whole-registry / per-entry fingerprint 与 surface checker assertions。

实现可使用 derive attribute连接 internal Rust error 和 generated DTO。直接同名字段由生成代码读取；
需要脱敏/归并时，source owner 只负责生成 typed DTO，不能手写 Skiff field map。

Artifact/wire 演进同时完成：

1. Compiler 把 `{ registryId, registryVersion, fingerprint }` exact descriptor写进PackageArtifact root和
   bytecode v7 header，并纳入architecture规定的identity preimage；Package Local ABI与ServiceProtocol
   identity不变。
2. Runtime binary/session registration声明自身 fingerprint；loader拒绝不匹配 artifact，Router只把 build
   路由给 matching runtime session。
3. `PlatformError` wire按canonical exact shape改为generated projection key + entry fingerprint +
   nonempty、至多64-KiB canonical payload，runtime frame hard cut到v5且无dual reader。
4. Outer envelope 合法但 key/fingerprint 未知时固定为 opaque service cause；本地不能 catch，仍可原样
   forward。
5. 本地已知 key/fingerprint 但 payload codec失败时产生 protocol failure；provider/local encode 在 envelope
   固定前失败时才生成固定 `InternalError`。
6. 同 key/identity 不原地修改 schema；字段 contract变化使用新 key/identity或显式全栈 hard cut。
7. Rolling upgrade允许不同whole-registry fingerprint的runtime session并存；release/artifact仍引用旧
   fingerprint时保留matching runtime，Router不得把它路由到新registry session。没有release/artifact引用后
   才能回收旧runtime/registry。

唯一 compiler-owned generator command 提供 `--check` gate。Generated projection key、entry descriptor 和
fingerprint 放在 compiler/runtime 已共同依赖的 `artifact-model`；runtime typed DTO 与 wire codec 放在
`runtime/request-contract`，只引用同一 generated key/descriptor。Generator 可以写多个职责不同的输出文件，
但 catalog parser、key registry 和 schema descriptor 只能生成一次，不得在消费 crate 中重建。

完成标准：

- 修改 `.skiff` type 后未刷新 generated source时 `--check` 失败；catalog没有可供双写的 field schema。
- Catalog 引用不存在、未公开或不满足 platform payload closure 的 type时 generator失败。
- Runtime 不再通过手写 string symbol重建 generated identity。
- Artifact/runtime fingerprint不匹配在 load/route前拒绝。
- Bytecode v7、identity generation 5、PackageArtifact v15/build v14和runtime-frame v5的schema snapshot、
  identity preimage与old-version reject测试通过。
- Unknown-valid projection变 opaque；known-malformed payload变 protocol failure；local/provider encode failure
  变 fixed `InternalError`，三条路径有互斥测试。

### M4 — Migrate existing platform errors

状态：Pending。

按现有 user-visible behavior 分批迁移：

1. collection index/missing-key opcode failure；
2. `TimeoutError`；
3. config/bytes/number/JSON/time decode；
4. DB conflict/constraint/decode；
5. file/HTTP/WebSocket；
6. service provider unavailable/protocol；
7. Package-owned `std.resource.ResourceError` 保持 Package public typed path，不错误迁入 builtin
   platform registry。

每批工作：

- 在 catalog 登记 projection key 到 exact public type 的关联，不复制字段 schema。
- source operation产生 dedicated typed projection DTO。
- eval使用统一 generated materializer创建 `RequestException`。
- service channel使用 generated codec导入/导出。
- collection/opcode 路径把 hand-written `Catchable { identity: &'static str }` 改为 generated
  `PlatformErrorProjectionKey` 与 generated payload builder，并让 compiler emission、verifier、linked
  bytecode、VM/loader共同验证 registry fingerprint。
- 删除该批在 artifact-model/compiler/boundary/native/capability/eval/host 中重复的 JSON field match和
  symbol string。
- 外层 error 语义未改变时改为 wrapper/delegation；需要改变语义时保留显式 conversion。

完成标准：

- 同类 payload在所有 crate都引用canonical Skiff type IR生成的同一DTO/materializer，不存在第二份
  field/schema owner。
- 当前 exact catch正例、错类型不匹配负例与已定义 user-visible behavior不变；service wire fallback按
  M3 明确收敛为 local/internal、inbound protocol、unknown opaque三条路径。
- Collection errors覆盖 source site、exact catch、service round-trip和VM invariant/index错误不得伪装成
  public collection error的负例。
- `RuntimeErrorPayload.code` 相同但不具备 projection 的负例继续成立。

### M5 — Explicit failure-site routing and operation admission

状态：Pending。

工作：

1. 为 error promotion 引入显式 failure-site context，至少区分：
   - control/load；
   - ingress before handler；
   - active request call site；
   - provider egress after handler；
   - durable/background platform work；
   - internal stop。
2. `active request call site` variant 必须携带 runtime-owned `ContinuationProjectionGuard`；guard至少绑定
   request generation、lane/resume owner、call site和仍未settle的 continuation state，不能由 error
   producer 构造或从 source site推断。
3. 每个 std/native/service/opcode operation 使用 closed
   `(operation, projection key, semantic failure class, phase)` admission policy，default deny。
4. Source semantic adapter负责把具体 failure class/phase转成 generated DTO；whole Rust error type、内部
   code 或 generic source error不能直接取得 projection capability。
5. `promote_call_site_error` 必须在 materialization前校验 guard与operation admission。错误 outcome可以
   明确 no-effect、effect-already-visible或outcome-unknown；catchability不额外承诺rollback/safe retry。
6. Egress/service export不能对从未在 active call site投影的 provider-internal Rust error进行“补投影”。
7. Ingress/control/background owner即使拿到 generated DTO也不能构造 request-local exception。
8. Timeout winner之后的late response、concurrent loser、已结束stream consumer和过期async resume只进入
   bounded diagnostic/terminal path，不借仍存活的request heap创建exception。

完成标准：

- 同一 synthetic projectable error在 active admitted call site可 catch，在 ingress/control/background
  matrix中不可 catch。
- 只有 source site/request heap但 guard缺失、generation/lane不匹配或continuation已settled时，
  materializer fail closed且不创建 `UserException`。
- Operation policy不能用开放 bool/string从外部输入；必须来自 compiler/runtime closed metadata。
- Existing HTTP/service/WebSocket timeout/transport errors在其reference允许时仍可catch，即使remote effect
  outcome unknown；测试同时证明catch后不会自动retry或声称rollback。
- `task.submit` definite rejection与ambiguous acceptance继续不可catch。
- Internal cancellation在所有 site保持 terminal。

### M6 — Recoverable public projection capability

状态：Deferred until first production admission。

M1–M5 不需要等待本里程碑。现有 platform errors 已足以验证 catalog/codegen/admission infrastructure；
不得只为测试基础设施而发布一个没有 production semantic owner 的 std error type。

工作：

1. 先选定一个真实 source operation、具体 semantic failure class/phase，并在对应 reference/architecture
   contract 中说明 error outcome、effect certainty 和 catch 后可继续语义。没有这个 owner 时保持 deferred。
2. 把该 operation 的 recoverable failure 显式分成 projectable value rejection 与 platform integrity
   failure；不能按 `RecoverableBoundaryError` whole type 或 code allowlist准入。
3. 在 `std/recoverable.skiff` 增加：

   ```skiff
   type BoundaryError {
     message: string,
   }
   ```

4. 在 `std/api.yml` 导出 `std.recoverable.BoundaryError`，同步 reference surface和 compiler public
   symbol assertions。
5. 在 projection catalog 增加 recoverable entry，并生成 Rust DTO、catch identity、materializer和
   service codec。
6. Semantic adapter只把脱敏 message放入 generated DTO。Internal code、context、expected plan、node path和
   raw detail留在 restricted diagnostic；需要业务分支时新增专门 typed error，不公开 generic code string。
7. 只为第1步选择的 `(operation, projection key, semantic class, phase)` 设置 closed admission；其它
   operation继续default deny。
8. 明确保留以下 production negative：
   - `task.submit`：`platform-fatal`；
   - ingress handler前：`protocol-rejection`；
   - control/background：`background-terminal`；
   - 同一operation中的artifact/state/sealed-payload等integrity failure：`platform-fatal`；
   - provider内部未投影 failure：跨 service变固定 `InternalError`。

完成标准：

- Generated payload exact 为 `message`，没有 internal code、`details`、stack、correlation或内部 context。
- 选定的 production call site可以 `catch<std.recoverable.BoundaryError>`，并且reference已说明effect
  certainty；catch本身不触发自动retry。
- 所有未准入 production call site仍不可捕获。
- 同一 Rust error/code 的 integrity failure负例保持不可捕获。
- 未捕获 admitted error可以跨 service保留 exact platform identity；provider encode失败、inbound
  known-malformed和unknown-valid分别走InternalError、ProtocolError和opaque forwarding。

### M7 — Deep source error conversion cleanup

状态：Pending。

工作：

1. 删除 generic `serde_json::Error`、BSON、Mongo、HTTP parser error的 public projection能力。
2. 在 source operation adapter建立专门 semantic wrappers：
   - std JSON call -> generated JSON decode projection；
   - recoverable codec -> `RecoverableBoundaryError`；
   - artifact decode -> `InvalidArtifact`；
   - service/runtime wire -> protocol/internal；
   - ingress pre-handler -> gateway protocol response。
3. 合并 boundary/native/eval 中重复的 `decode_target_error_code()`；目标不是一个全局 string
   switch，而是 source operation在创建 error时选择 typed wrapper。
4. 底层 source error保留在 Rust chain供 restricted diagnostic使用，public message单独脱敏。

完成标准：

- Generic `serde_json::Error` 单元测试证明没有 catch identity。
- 同一 malformed JSON 在 std JSON、recoverable、artifact和 ingress fixture中进入四条不同、正确路径。
- Public error payload不包含 raw JSON、原始 parser message或secret。

### M8 — Remove transitional projection owners

状态：Pending。

全部 consumer迁移后：

- 删除 `WirePayload::catch_projection()` 或把 `WirePayload` 缩回纯 diagnostic/control DTO职责。
- 删除手写 `PlatformBuiltinErrorIdentity::from_symbol/symbol` table，改用 generated registry。
- 删除 artifact-model/compiler/opcode/VM 与 boundary/native/eval/host 中已迁移的 hand-written symbol、
  duplicate payload JSON matches。
- 保留 `RuntimeErrorPayload` 供 control/internal response使用，但命名/注释必须明确它不授予
  catchability。
- 更新 architecture/reference/implementation 实施状态，记录最终 owner；wire/header canonical contract必须已
  在M3完成，M8不重新决定，不保留旧 mapping compatibility。

完成标准：

- 反向搜索不存在 production hand-written platform error field map、opcode catch symbol或 symbol inference。
- 每个 projectable platform error恰有一个 catalog entry。
- 每个 operation恰有一个 closed admission owner。
- implementation status更新为完成，并列出最终聚焦验证结果。

## 6. Verification matrix

### 6.1 Model and compiler

- Catalog schema strict parse：unknown/missing/extra field拒绝。
- Catalog exact type引用必须存在、公开且schema-closed；catalog自身不存在field schema。
- Generated identity、payload DTO、catch leaves和service codec round-trip。
- `.skiff` schema mutation但未刷新generated source时generator `--check`失败。
- PackageArtifact/bytecode、runtime binary/session的registry fingerprint匹配；load/route mismatch fail closed。
- Runtime capabilities缺失/非法descriptor、Router跨fingerprint route与PackageArtifact/bytecode descriptor
  不一致都在dispatch前拒绝。
- Collection opcode只保存generated projection key，compiler/verifier/VM不含hand-written catch symbol。

### 6.2 Runtime projection

- Exact catch、nonmatching catch和rethrow。
- Projectable capability + denied site不能构造 exception。
- Guard有效的admitted active call site产生source/stack/correlation。
- Guard缺失、request generation/lane不匹配、continuation已settled时不创建exception。
- Local/provider encode failure、inbound known-malformed payload和unknown-valid projection分别进入
  internal、protocol和opaque路径，均不泄露detail。

### 6.3 Site matrix

- Control/load projectable error仍是 control failure。
- Ingress pre-handler decode仍是 external protocol response。
- Handler active call site按 admission投影。
- Provider egress不补投影内部 error。
- Background/scheduler error不回投旧 request。
- Timeout后late response、concurrent loser、stream consumer结束和expired async resume不回投旧
  continuation。
- HTTP/service/WebSocket outcome-unknown error按reference仍可catch，但不自动retry或承诺rollback。
- Cancellation/scope stop没有 ordinary payload。

### 6.4 Source conversion

- `std.json.decode` malformed input -> `std.json.DecodeError`。
- Recoverable inner JSON failure -> `RecoverableBoundaryError` diagnostic；只有operation semantic adapter选中的
  value-rejection class/phase可构造generated DTO。
- Artifact JSON failure -> invalid artifact/control failure。
- Ingress JSON failure -> gateway response。
- Generic nested provider JSON error -> internal，不能因 source type自动 catch。

### 6.5 Recoverable

- Large error clippy gate。
- M6 deferred期间当前std surface不存在`std.recoverable.BoundaryError`，所有production path保持deny。
- 首次激活后public payload exact为`message`；internal code和diagnostic context不进入public payload。
- 只准入一个有reference owner的production operation/class/phase；dispatch/ingress/control及同operation
  integrity failure保持negative。
- Admitted未捕获error跨service exact identity/correlation；未投影internal error变`InternalError`。

### 6.6 Service channel and observability

- `PublicTypedError`、`PlatformError`、`InternalError` 三variant strict round-trip。
- `PlatformError` projection-key grammar/长度、fingerprint format、nonempty/64-KiB payload limit和unknown field
  strict reject；old `builtinErrorIdentity` shape在runtime-frame-v5 hard cut后拒绝。
- Unknown-valid platform projection与其它opaque fixed error bit-equivalent forwarding。
- Known-malformed platform payload投影为caller-side protocol failure，不改写成provider InternalError。
- Provider local fallback保留当前cause correlation；caller-side protocol failure分配本地error correlation；
  unknown opaque保留原canonical trace/error bytes。三条路径都不得信任或泄露未验证metadata。
- Caller新建本地栈，provider完整栈只在 restricted telemetry。
- 每个新 exception只记录一次，catch/rethrow/import不重复记录。

## 7. Focused commands

具体 selector以实施时 `node scripts/verify.mjs --list` 和 Cargo test list 为准。不要在每个 milestone
都运行全量 `pnpm verify`。建议顺序：

```bash
cargo test -p skiff-runtime-request-contract --lib --no-fail-fast
cargo test -p skiff-runtime-model --lib --no-fail-fast
cargo test -p skiff-runtime-boundary --lib --no-fail-fast
cargo test -p skiff-runtime-eval --lib --no-fail-fast
cargo test -p skiff-runtime-host --lib --no-fail-fast
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only rust-quality
git diff --check
```

不要并发运行 Cargo。预计超过 30 秒的命令必须把 stdout/stderr 重定向到一个新建临时文件，并通过
单独 tail/poll 查看，避免不确定时重复启动。

影响 boundary/eval/request/host 或 service error channel 的 milestone 完成后必须按仓库约定执行
Agine chat smoke。若在语言 worktree验证 runtime/router 本身，使用独立 run dir；不要修改 main
worktree 的稳定 `.skiff-dev` 配置。

## 8. Commit and rollout boundaries

建议每个 milestone独立提交，最少保持以下边界：

1. M0 tests/audit freeze；
2. M1 recoverable boxing；
3. M2 diagnostic/projectability split；
4. M3 canonical carrier/version contract（docs only，原子更新三份architecture contract）；
5. M3 catalog/generated schema与hard-cut carrier实现；
6. M4 existing errors migration（可按 error family再拆）；
7. M5 site/admission；
8. M6 first production recoverable projection（只有触发条件满足时执行）；
9. M7 source conversion cleanup；
10. M8 legacy owner deletion与最终状态文档。

不得把 M1 clippy 修复与 M6 public language semantic change放进同一个提交。每个提交都必须说明：

- 改变了哪些 error owner；
- 哪些 call site仍 default deny；
- 是否改变 Skiff catch surface；
- 是否改变 service/external wire；
- 运行了哪些聚焦验证。

## 9. Completion criteria

本计划只有在以下条件全部满足时才能标记完成：

1. Architecture §10 的全部 invariants有自动化测试owner。
2. 所有 platform error payload shape由canonical Skiff type IR/codegen唯一拥有；catalog不复制字段schema。
3. 所有 actual projection经过runtime-owned continuation guard和closed operation/class/phase admission。
4. `RecoverableBoundaryError` 已缩小，且 projectable capability与production admission分离。
5. Generic deep source error不能自动取得 Skiff identity。
6. Internal terminal、user exception、fixed service failure没有被 generic diagnostic扁平化破坏。
7. Artifact/runtime registry fingerprint、unknown opaque forwarding和known malformed protocol path全部通过。
8. `bytecode-vm.md`、`package-service-contract-deployment.md` 与本文对registry authority、carrier、version和
   preimage的canonical描述一致，且是在M3 code前原子更新。
9. Service channel、ingress、background和observability负例全部通过。
10. 重复的手写 projection owner（包括opcode/VM symbol）已删除，无旧registry compatibility fallback。
11. 对应 reference surface与最终代码一致；若M6仍deferred，reference不提前出现reserved type。
12. 聚焦 compiler/runtime/rust-quality、chat smoke 和 `git diff --check` 全部通过。
