# MAP5：Phase 5 rolling execution map

> Status: active; revision 20; recovery epoch `r1`; canonical cross-document convergence in progress after fresh Design FAIL; no implementation/proof lane complete
>
> Phase Contract: [`phase-5-typed-host-effects-resources-streams.md`](../phases/phase-5-typed-host-effects-resources-streams.md), Amendment r2
>
> Exact baseline commit/tree: `e643d11fe763200c40b49e24ca922321799278f0` / `c511de9675f6d1a70fd5c995119f44be311cbc4e`
>
> Upstream receipt: [`results/phase-4.md`](../results/phase-4.md)
>
> Target integration branch/worktree: `codex/bcvm-p5-integration-r1` / `/Users/geek/workspace/skiff-bcvm-p5-integration-r1`

## 1. Recovery epoch

`r1` 从 exact Phase 4 main baseline 新建；先 cherry-pick 本 Contract/MAP Amendment 的 portable docs commit，
不继承旧 integration 的 partial-merge ancestry。以下状态全部只是 audit/salvage source，不是已完成 lane，任何
commit 都不得整笔 merge/cherry-pick；新 owner 只能在 r1 worktree 中按当前 Contract 逐 hunk 重做并重新证明。

| 旧 branch/worktree | 已提交 tip / dirty state | r1 disposition |
| --- | --- | --- |
| `codex/bcvm-p5-integration` / `skiff-bcvm-p5-integration` | `8931e522b068a0190c82ebf31bb7d3d86f777ea2`；`runtime/linker/src/bytecode/link/capability.rs` dirty | abandon ancestry；仅供审计，dirty fix 不转移 |
| `codex/bcvm-p5-compiler` / `skiff-bcvm-p5-compiler` | clean `6f945f0b2191fba1129eefb6dc960dc8323db74d` | C5 仅 salvage exact-negative test intent；重做 exact surface与 affine composite |
| `codex/bcvm-p5-verify` / `skiff-bcvm-p5-verify` | `bce17a6c73da2f84342b2f6d93276e8e1bf158a6`；两文件 dirty | discard context-wide/`allows_stream` relax和 string gate；重做 typed image transport |
| `codex/bcvm-p5-kernel` / `skiff-bcvm-p5-kernel` | `8f9a920e43bb569ba3b0988bb4f29f96cba222e0`；10 tracked + 1 untracked dirty | 仅 salvage Phase 4 pending helper、capacity=1 supervisor与 exact-handle意图；ResourceTable/JSON provider/dummy handle/string dispatch 全部 redesign |
| `codex/bcvm-p5-proof-gate` / `skiff-bcvm-p5-proof-gate` | clean `4c8dec19cc35761c675adf80da41e0abb3a6d2df` | 仅 salvage Gate/evidence scaffold；所有 Rust proof body 重写，旧 PASS 无效 |

旧 Phase 5 output/evidence 全部跨 epoch 失效。r1 的 first proof attempt 必须在任一 production lane join 前，用新
真实断言矩阵一次 `--no-fail-fast` 记录 nonzero/non-skip expected-red；Phase 1–4 regression 子集应保持绿。

## 2. Gate-map 预调查

1. A5 authority/schema：pinned registry产生 accepted closed executor identity；artifact lifecycle表达 privileged
   recursive affine composite/field take，不自报 execution authority。
2. C5 source/admission/emission：只接受 sleep regression + exact HTTP request/stream及合法 placement；输出 exact
   callsite、take、resume、transfer/drop facts。
3. V5R atomic link/image：linked target运输非字符串 identity；同一 atomic constructor只消费 compiler/artifact
   facts并闭合 bounded structural references；不存在独立 verifier stage，也不得用 context/type/opcode重建 semantics。
4. K5 VM/scheduler/request：scheduler-private ResourceTable、Phase 4 shared pending/root/inventory、first-poll
   Ready/Pending、heap-free completion、affine take/drop、two-handle routing与capacity=1 backpressure。
5. H5 host/session：复用 production HTTP lower和 capability API；每 request 注入 ResourceTable-backed
   `StreamRuntimeApi`；server-stream response在 runtime WebSocket writer前具有 bounded/flush-aware backpressure。
6. P5G proof：同一真实 fixture沿六 stage sentinels并穿 actual Router HTTP↔runtime WS↔RuntimeHost↔outbound TCP
   full chain；不手造 image/executor/handle/frame。
7. Router production：默认无改动；现有 production gateway/dispatcher/session/chunked writer是被测 consumer。

### 2.1 Shared design receipt

2026-08-14 的独立 shared-design review 对 r1 authority/lifecycle 决定给出 **PASS**。affine composite 采用
stack consume-whole `TakeDenseField(shape, ordinal)`（stack `1 → 1`），不引入 slot partial mask：compiler 在
take 前对源 slot 执行 `TakeSlot`，并把该 slot 的 source fact 置为 `Moved`；VM 原子取得 exact field owner后立即
按 recursive plan drop aggregate remainder。heap 的 generation/owner-bound take guard 防御重复或伪造 take，
但不成为第二 lifecycle authority。slot 已整体 moved，所以 verifier 沿现有 stack/control-flow owner状态证明，
无需新增 partial-state merge或 stack-map事实。

### 2.2 Schema epoch pin

A5 的 artifact schema v8 / linked candidate schema v5 把 privileged field facts 变为 mandatory；所有 producer、
canonical constructor、identity pin 与 full-publication assertion 必须在同一 r1 epoch 更新。语言尚未发布，因此
不保留旧 schema compatibility；新增的 compiled/driver 测试文件只负责 schema pin 与完整 publication regression，
不扩大 C5 semantics。

### 2.3 Contextual output lifecycle transport

`TakeDenseField` 的 stack output 虽只产生被取 field，仍必须在 exact package、specialization 与 substitutions
上下文中解析该 field 的 privileged recursive lifecycle plan。V5 因此拥有
`runtime/linker/src/bytecode/stack_map/values.rs`，把该调用点从 generic `plan_for_concrete_type` 收敛到 contextual
`plan_for_concrete_type_at(...)`；这是 output lifecycle fact 的 transport，不新增 slot partial/taken mask、CFG
state 或 stack-map authority。focused regression 继续放在已授权的 `runtime/linker/src/bytecode/tests/**`；当前没有
需要新增或授权的 `runtime/linker/src/bytecode/stack_map/tests/**` 路径。

### 2.4 Router flush export companion

H5 的 public `RouterWriteFailure` / `RouterStreamFrameFlush` 定义在 private `outbound_control` module，production
router session 与 K5 producer 跨 crate 使用时必须由 `runtime/capability-context/src/lib.rs` 机械 re-export。
该 companion 只暴露同一 non-cloneable flush receipt/failure vocabulary，不新增 execution、terminal、pending 或
stream authority，也不扩大 Phase 5 支持面。

### 2.5 Linked authority schema comment pin

linked candidate schema 已在 r1 同 epoch 升至 v5、artifact bytecode schema 升至 v8；
`runtime/linked-bytecode/src/authority.rs` 仍残留“bytecode schema v7”的文档注释。V5 获准只把该注释机械 pin 到
v8，使 authority 文档投影与 canonical schema epoch 一致；不修改 authority 类型、pin、validation 或执行语义。

### 2.6 Source value-transfer fact producer

C5 的 production `source_value_transfer_facts_for_units` 必须从已 admit MIR 的 exact `external_refs` 读取 canonical
`PackageId`、symbol path 与 nonempty ABI，并只用 bit-exact `package_type_records[(packageId, path)]` 生成对应
`SourceValueTransferNominalFact`。`compiler/driver/pipeline/bytecode_lane.rs` 因而属于 C5 producer write set；
emission 不得从 nominal/type name、record shape或 std 特判猜 lifecycle，本 amendment 也不放宽其它 driver 语义。

### 2.7 Mandatory HTTP-port regression callsites

K5 给 `BytecodeRequestExecutionInput` 增加 mandatory `http_client` 后，Phase 2/3/4 既有 proof-support composition
必须机械补 `http_client: None`，否则 `runtime/host` crate tests 无法编译。H5 仅获准修改这三个旧 callsite 的
新字段初始化，不改变其 fixture、assertion、proof语义或 authority；Phase 5 production 的五个 composition
callsite 必须显式传 `Some(typed port)`，不得借用 regression 的 `None` fallback。

### 2.8 Phase 4 typed-view compatibility

V5 删除 production raw `DeploymentExecutionImage::host_effect_adapters()` accessor 后，Phase 4 emission→link
sentinel 必须机械迁移到 `HostEffectAdapterIndex -> image.host_effect_target(index)` opaque view。H5 只获准在
`phase_4_vcp_tests.rs` 保持“exactly one pinned Sleep target + same linked signature”的原断言；不得改 Phase 4
fixture/proof语义、production、authority，不得恢复 raw slice/string accessor或放宽 missing/out-of-range rejection。

### 2.9 Host test dependency lock projection

Phase 4 typed-view/P5G typed-stage tests需要 `runtime/host/Cargo.toml` 的 direct dev-dependency
`skiff-runtime-linked-bytecode`。Cargo resolver 对根 `Cargo.lock` 的唯一预期变化是把同名 workspace package加入
`skiff-runtime-host` 的 dependency list；H5 获准提交这一 mechanical lockfile companion，禁止版本、checksum、
其它 package 或 dependency list 漂移，不因此扩大 production dependency/authority。

### 2.10 Canonical gateway authority before bytecode lane

C5 的 compile core 必须先构造并验证内存中的 canonical unattached `PackageArtifact`（不得写 store/release
pointer），再复用唯一 `project_http_gateway_after_package_validation` 得到 exact rawHttp/serverStream typed
handler/protocol authority并传给 bytecode lane；普通 package 得不到该 authority。execution attach后的正式
deployment projection必须与同一 projector一致。`compiler/driver/pipeline/mod.rs` 因而属于 C5 producer write set；
禁止从 config/string/`Stream` shape另建 gateway validator或 authority。

### 2.11 Mandatory response ceiling handles

K5 给 `BytecodeRequestExecutionHandles` 增加 mandatory `max_response_bytes`。H5 已授权的五个 production
composition callsite 必须传各自现有 bootstrap response ceiling；Phase 2/3/4 proof-support 三个 callsite 只获准
补固定测试值。该机械字段迁移不改变旧 fixture/assertion/proof语义；K5 request tests 继续归 K5 既有写集。

### 2.12 Canonical reachable package closure resolver

C5 的 reachable package closure 只由 `compiler/driver/authoring.rs::resolve_reachable_package_closure` 拥有；
该 helper/validator 可改为 `pub(crate)` 并在同一处加固：每条 requirement 必须唯一匹配 exact coordinate、local
ABI 与 optional expected build，candidate 必须通过 artifact identity validation；只有 loaded candidates 中没有该
coordinate 时才允许 existing store resolver 的 read fallback。pipeline 必须删除全部复制的 BFS、requirement
matcher 与 artifact-identity validator并复用该 helper；formal deployment/authoring 继续走同一 helper。
`http = None` 的 ordinary `compile_package` 不解析 gateway closure且行为不变。不得修改 package
publication、pointer 选择/CAS、store write/record layout 或 authoring input 语义。

### 2.13 Claimed pending-wake resume authority

K5 必须删除或私有化 public `BytecodeScheduler::resume_from_suspended` standalone `RootEscrow` 路径；该入口可由
caller 自行注入 escrow，绕过 mandatory retained `RequestResourceRootPin` 与 claimed/mapped wake guard。现有三个
legacy direct-resume callsite 必须在不改变测试意图的前提下迁移到真实
`PendingRegistry → PendingWake → ClaimedPendingWakeGuard → MappedPendingWakeGuard` 路径；其中外部 scheduler test
的两个 callsite 由本 revision 新增写集拥有，`bytecode.rs` 内部 callsite 已在 K5 原写集。guard 的 map/commit
实现必须保留 `S: VmRootSource` 与 `O: VmRootSource` bounds，使 suspended owner与 settlement outcome在同步
materialization/resume commit 全程仍是一个可枚举、不可拆分的 root source；不得留下 direct escrow fallback。

### 2.14 Request server-stream module boundary

K5 将 bytecode server-stream request runtime 从已约 3.4k 行的 `bytecode_ingress.rs` 拆到
`runtime/request/src/bytecode_server_stream.rs`。新文件只承载 private server-stream runtime/supervisor、exact event
decode、reservation → transport-writer first poll → ACK materialization、failure/termination mapping及对应 unit tests；
`bytecode_ingress.rs` 只保留 entry validation/start wiring、`PendingOutcome` 分派与最终 projection。public transport
DTO/trait 继续只在 `bytecode_host_effects.rs`；central capacity/sequence/cap/terminal authority继续只在
`runtime/scheduler/src/resource.rs`。该职责拆分不得新增 registry、terminal/cap state或 H5 transport logic。

### 2.15 Exact server-stream branch contextual field typing

C5 source inference只在 authoritative stream-result target 是 exact canonical
`skiff.run/std::std.http.HttpResponseStreamEvent`、statement 是 `serverStream` producer 的 direct `Emit`、union
discriminant精确选择 `tag = "start"` branch时，才允许其 direct `headers: []` field从该 branch 的 canonical field
fact取得 `Array<std.http.HttpHeader>`。candidate selection必须先以 exact target/branch/field authority闭合，再递归
materialize该 empty array expression fact；不得把 contextual array typing扩到普通 array、local/identifier slot、非
direct field、其它 union/branch/field或 non-serverStream producer。lowering只消费 source-owned materialization并
独立重验 exact branch field与 child expression type，不按名字/shape重新推断。

因此 C5 同时删除 `admission/server_stream.rs` 的 `builtin_calls`、`admit_response_intrinsics`、
`admits_builtin_call`，以及 `admission.rs::admit_call` 对 generic type args / builtin target 的对应旁路；production
fixture使用 direct `headers: []`，不再用 `Array.empty<std.http.HttpHeader>()` 伪造 intrinsic capability。不得向
artifact registry、linked image或 VM 增加 static intrinsic authority。

### 2.16 Independent verifier retirement hard cut

[`DEC1 Amendment r2`](../decisions/dec1-executable-image-authority.md#amendment-r2-2026-08-14-retire-the-independent-production-verifier)
与 Phase Contract Amendment r2 删除独立 production `runtime/bytecode-verifier` stage/crate。compiler是 source
semantics 唯一 authority；linker atomic `DeploymentExecutionImage` constructor最多承担 private bounded
decode/index/CFG/stack/slot/call/resume consistency和 statement schedule的 exact fact-to-index mapping。linker只解析/
消费 compiler-emitted facts及其 pinned registry references；缺 fact就失败，不得从 opcode、name、context、registry
membership或 type/shape重推 semantic admission、effect、placement、lifecycle或 source attribution。

这是一个不可拆的 hard-cut checkpoint：删除 workspace member/crate、`ExecutableFacts`、
`verify_executable_facts`、verifier-owned consumer types、所有 production dependency/import、旧 tests/Gate selector，
并把 structural cases移到 linker image-construction tests、semantic cases留给 A5/C5已有 test owner。禁止 alias、shim、
feature-gated legacy、forwarding crate、test-only old path或新旧双跑。`VerificationLimits`收敛为 image/linker-owned
construction limits；schedule/resume/effect/constant views只能由 complete image窄暴露，不得形成第二 facts bundle。

现 `codex/bcvm-p5-v5-r1` / `skiff-bcvm-p5-v5-r1` 停在 `cb6de45aed3d80a510f9dccbc6f051c6a0112be1`
并含 verifier semantic reconstruction与 linker 混合 dirty work。该 worktree立即降为 audit-only：不提交、不整笔
salvage；只能把 structural negative-test intent逐项重写到 V5R，source-semantic intent返回 A5/C5。V5R 从本 docs
commit后的 clean integration HEAD 新建。r1 §2.1 的 PASS 只覆盖原 affine/resource lifecycle choice，不覆盖本次
stage删除；V5R 开码前必须取得新的 independent Design PASS。

V5R hard cut期间，下表中与 K5/H5 重叠的 manifest、VM/request test与 Phase 4 compatibility文件全部由 V5R
临时独占，K5/H5 必须停止写入；atomic checkpoint join并 rebase后才恢复各自原 semantic owner。P5G只拥有 Phase
4/5 selector与 Phase 5 stage-test companion，并与 V5R 同一 checkpoint join，不能先提交指向不存在 target 的
中间 Gate。损坏 artifact可在 construction失败，或在明确延迟检查点产生 checked safe request failure；两条路径
都必须证明无 UB/process crash/越界/partial image/pending-root-resource leak。

### 2.17 Canonical architecture convergence

对 `4cc8dd8036b99ab2227b455c71ec6d63375e54f3` 的 fresh Design review 只因 canonical
[`doc/architecture/bytecode-vm.md`](../../../architecture/bytecode-vm.md) 仍要求独立 post-link semantic verifier而
**FAIL**；implementation decision本身没有新增 objection。rev17 将该 canonical architecture 收敛为 compiler
source-semantics唯一authority、保留§4.1 pre-link structural validation，并把必要bounded structural closure与
image-owned schedule/resume/effect/constant窄view统一放进 atomic `DeploymentExecutionImage` construction。
architecture不记录crate删除/selector迁移步骤；这些仍只属于 DEC1/Contract/MAP。

D5 是本 revision 唯一 docs owner，exact write set见下表。该 docs commit必须先接受一次全新 independent Design
review；PASS前不得创建 V5R code worktree或恢复 K5/H5 overlap。旧 `4cc8dd80` review结果不能升级为PASS复用。

### 2.18 Fresh Design review cross-document closure

对 `f8370b3376632e0eae7328752ad991bb0787c5a1` 的 fresh Design review 仍为 **FAIL**。该 commit 已正确删除
`bytecode-vm.md` 中的独立 semantic verifier authority，但 canonical lazy-load、deployment、DB、interface、
tail-call 与 reference 文档仍保留 post-link semantic verification 或 verifier-owned source semantics；同一文档还
残留 bytecode schema v7 / ISA v4，而 r1 当前 epoch 已是 schema v8 / ISA v5。

D5 因此临时扩展为下表列出的完整 cross-document closure。修订必须保持 compiler-only source semantics、single
atomic image mint、construction-local bounded structural closure 与 checked runtime safe failure，并明确拆分 tail-call
的 relocatable compiler facts 与 deployment concrete-specialization resolution。修订完成后必须由另一名全新
independent Design owner 从 clean exact HEAD 重新审查；PASS 前 V5R/K5/H5 继续暂停。

### 2.19 Active process contract and nested VM companion

Hard-cut inventory发现active convergence `README.md`仍把薄verifier列为未决选项，唯一Phase runbook仍固定
`link→verify→scheduler` sentinels；两者必须与r2一并收敛，不能让执行流程保留第二种现行解释。另有
`runtime/vm/tests/vertical/k2_scalar_core.rs`直接持有verifier-owned fixture命名与helper，属于V5R consumer
migration不可分割的nested test companion。D5/V5R exact write set按下表扩展；历史Phase 0–4 receipts/reviews仍不
做全局改写。

### 2.20 Observability, DB and current-version canonical closure

对 clean exact HEAD `01fdc6f801875070da8115b6ab053addb91f0195` / tree
`48b724bd21d4209b6f5de34d5bea8719df3b6dd9` 的下一次 independent Design review 仍为 **FAIL**：active
observability architecture/reference仍把 deployment image分解为 `load / link / verify` 并保留 semantic-verification
rejection类别，DB reference仍写 image-local `link/verification`；runtime-error canonical checkpoint仍把当前 bytecode
描述为 schema v7 / ISA v4 与尚未落地的 production cutover，和本 epoch 已落地的 schema v8 / ISA v5不一致；active
README 的 Phase 5状态也仍指向已被 r2 supersede 的 Contract Amendment r1。

D5 因此继续扩展到下表新增的 observability、DB 与 runtime-error canonical 文件。修订必须把 image telemetry
收敛为 load / single atomic construction / cache / rejection，把 rejection分类限定为 decode、bounded structural
validation、exact reference closure、resource limit与 timeout；DB binding只能消费 compiler-emitted schema facts并在
同一 atomic constructor做 exact referential closure；runtime-error 文档必须明确 v7/v4 projection-registry cut之后
已由 Phase 5 v8/v5 hard cut增加 `TakeDenseField` 与 privileged affine-composite carrier，不能再声明 production cut
尚未发生。完成后仍须由全新 independent Design owner审查 clean exact HEAD；PASS前不创建 V5R，也不恢复K5/H5。

## 3. Lanes、唯一 write sets 与 rolling join

表内 write set 是本 Phase 唯一文件清单权威；lane 内 focused unit test 可放在所列 module 的现有/新 test
子文件。任何扩展先停止、上报并修改本 MAP。每个 worktree 同时只有一个 write owner。

| Lane / status | Branch / worktree | 唯一 write set | Depends / join |
| --- | --- | --- | --- |
| D5 canonical architecture + cross-document closure / fresh review FAIL, revision active | `codex/bcvm-p5-integration-r1` / `skiff-bcvm-p5-integration-r1` | `doc/architecture/{bytecode-vm.md,tail-call-execution.md,runtime-lazy-load-deployment.md,package-service-contract-deployment.md,db-capability-architecture.md,any-interface-value.md,observability-requirements.md,runtime-error-to-skiff.md}`；`doc/reference/{interface.md,any-interface.md,static-semantics.md,runtime.md,std-surface.md,observability.md,db.md}`；`doc/implementation/bytecode-vm-convergence/{README.md,runbook.md}`；`doc/implementation/bytecode-vm-convergence/decisions/dec1-executable-image-authority.md`；`doc/implementation/bytecode-vm-convergence/phases/phase-5-typed-host-effects-resources-streams.md`；`doc/implementation/bytecode-vm-convergence/tasks/phase-5-execution-map.md` | user architecture ruling；docs-only checkpoint；new clean-HEAD independent Design PASS before V5R |
| A5 authority + affine schema / ready | `codex/bcvm-p5-authority-r1` / `skiff-bcvm-p5-authority-r1` | `artifact-model/src/host_effect_registry/{contract.rs,registry.rs,tests.rs,mod.rs}`；`artifact-model/src/native_value_lifecycle/{contract.rs,registry.rs,tests.rs,mod.rs}`；`artifact-model/src/value_lifecycle_policy/**`；`artifact-model/src/bytecode/{dto.rs,opcodes/**,validate/{instructions.rs,plans.rs},tests/**}`；`artifact-model/src/lib.rs`；`artifact-identity/src/tests/mod.rs`（mechanical schema identity pin）；`runtime/native-contract/src/http_targets.rs`（仅复用/集中 canonical constants，不得成为第二 bytecode authority） | docs; join 1，首个非文档 commit |
| C5 compiler / ready after A5 API | `codex/bcvm-p5-compiler-r1` / `skiff-bcvm-p5-compiler-r1` | `compiler/source/src/{value_transfer/**,callable_effects/**}`；`compiler/source/src/expression_type_model.rs`；`compiler/source/src/expression_type_model/{assignability.rs,expression_assignability.rs,materialization.rs,object_materialization/tests.rs}`；`compiler/lowering/src/mir/**`；`compiler/lowering/src/function_lowering.rs`；`compiler/lowering/src/function_lowering/{object_literal.rs,object_literal/fact_validation.rs}`；`compiler/lowering/src/source_file_lowering/tests/object_materialization.rs`；`compiler/emission/src/bytecode/{admission.rs,admission/**,constants.rs,emitter.rs,functions.rs,plans.rs,mod.rs,tests/**}`；`compiler/compiled/src/bytecode_handoff/tests.rs`（schema pin/full publication regression only）；`compiler/driver/authoring.rs`；`compiler/driver/authoring/tests.rs`（only if canonical resolver focused regression is required）；`compiler/driver/pipeline/mod.rs`；`compiler/driver/pipeline/bytecode_lane.rs`；`compiler/driver/pipeline/bytecode_lane/tests.rs`（schema pin/full publication regression only） | A5; join 2a；Phase 5 admission放新子模块，避免继续膨胀单文件 |
| V5R atomic image + verifier retirement / redispatch after Design PASS | `codex/bcvm-p5-image-r1` / `skiff-bcvm-p5-image-r1`（从本 docs commit后的 clean integration HEAD新建） | typed transport：`runtime/linked-bytecode/src/{authority.rs,targets.rs,targets/**,plan.rs,candidate/**,tests/**,lib.rs}`（`authority.rs` 仍仅 mechanical schema-comment pin）；hard cut：`Cargo.toml`、`Cargo.lock`、`runtime/bytecode-verifier/**`（delete）、`runtime/linker/Cargo.toml`、`runtime/linker/src/{lib.rs,bytecode/**}`、`runtime/linker/tests/phase_1_contract/**`；consumer/dependency migration：`runtime/host/Cargo.toml`、`runtime/host/src/loader/bytecode_admission.rs`、`runtime/host/src/host/request_entry/phase_4_vcp_tests.rs`、`runtime/request/Cargo.toml`、`runtime/request/tests/bytecode_request.rs`、`runtime/vm/Cargo.toml`、`runtime/vm/src/{fiber.rs,statement.rs}`、`runtime/vm/src/fiber/{projection_tests.rs,tests.rs}`、`runtime/vm/tests/vertical.rs`、`runtime/vm/tests/vertical/k2_scalar_core.rs`、`runtime/package-test/{Cargo.toml,src/lib.rs}`、`test-runner/{Cargo.toml,src/runtime_execution.rs}`；structural registry：`scripts/check-runtime-crate-dag.mjs`、`scripts/lib/verify-rust-subjects.mjs` | A5+C5 facts + new independent Design PASS；atomic join 2b with P5G selector companion；old V5 dirty tree audit-only |
| K5 Resource/Pending/VM kernel / paused for V5R hard cut | `codex/bcvm-p5-kernel-r1` / `skiff-bcvm-p5-kernel-r1` | `runtime/scheduler/src/{owner_inventory.rs,pending.rs,resource.rs,root_escrow.rs,stream.rs,stream_driver.rs,bytecode.rs,lib.rs}`；`runtime/scheduler/tests/bytecode_scheduler.rs`（legacy direct-resume migration only）；`runtime/model/src/{vm_heap.rs,vm_value.rs,lib.rs}`；`runtime/request/Cargo.toml`；`runtime/request/src/{bytecode_ingress.rs,bytecode_server_stream.rs,bytecode_host_effects.rs,vm_heap.rs,execution_budget.rs,response_event.rs,outbound.rs,lib.rs}`；`runtime/request/tests/bytecode_request.rs`；`runtime/vm/src/{control.rs,fiber.rs,lifecycle.rs,lib.rs,fiber/tests.rs}`；`runtime/vm/tests/vertical/**` | V5R image API; join 3；§2.16 overlap在 hard-cut期间归 V5R，rebase后恢复；ResourceTable只在 scheduler，禁止写 `runtime/model/src/resource.rs` |
| H5 production host/session composition / paused for V5R hard cut | `codex/bcvm-p5-host-r1` / `skiff-bcvm-p5-host-r1` | `Cargo.lock`（通常只允许 `skiff-runtime-host` dependency list机械投影；§2.16 hard cut临时归 V5R）；`runtime/capability-context/src/{http.rs,lib.rs,outbound_control.rs}`；`runtime/transport/src/response_mapper.rs`；`runtime/transport/src/response_mapper/tests.rs`；`runtime/host/Cargo.toml`；`runtime/host/src/capability_context/http.rs`；`runtime/host/src/host/{mod.rs,runtime_host.rs,http_client_runtime.rs,http_runtime/**,http_response_ceiling.rs,request_supervisor.rs,router_session.rs,router_session/**,bytecode_capability_adapter.rs}`；`runtime/host/src/host/request_entry.rs`；`runtime/host/src/host/request_entry/{assembly.rs,assembly_wire.rs,websocket_jsonrpc.rs,bytecode_host_effects.rs,server_stream.rs}`；`runtime/host/src/host/request_entry/phase_{2,3,4}_proof_support/request_composition.rs`（仅 mechanical mandatory fields：`http_client: None` + fixed `max_response_bytes`）；`runtime/host/src/host/request_entry/phase_4_vcp_tests.rs`（通常仅 typed-view compatibility；§2.16 hard cut临时归 V5R） | K5 public port + V5R image API; join 4；§2.16 overlap在 hard-cut期间归 V5R，rebase后恢复；复用 existing lower |
| P5G proof + Router composition + Gate / V5R selector companion active | `codex/bcvm-p5-proof-gate-r1` / `skiff-bcvm-p5-proof-gate-r1` | `runtime/host/tests/bytecode_vm_phase_5.rs`；`runtime/host/tests/bytecode_vm_phase_5/**`；`runtime/host/tests/fixtures/bytecode-vm-phase-5/**`；`router/tests/bytecode_vm_phase_5.rs`；`router/tests/bytecode_vm_phase_5/**`；`scripts/lib/bytecode-vm-phase-4-contract.mjs`（verifier selector hard-cut only）；`scripts/lib/bytecode-vm-phase-5-*.mjs`；`scripts/run-bytecode-vm-phase-5-gate.mjs`；`scripts/tests/bytecode-vm-phase-5-*.mjs`；`scripts/lib/{verify-cli.mjs,verify-plan.mjs,verify-selector-graph.mjs}` | V5R exact replacement test names；selector companion与 V5R atomic join 2b；final proof join 5 after H5；不得写 production |
| R5-fix conditional / not dispatched | none | `∅` | 只有真实 P5G 证据定位 Router defect 后，先修 Contract/MAP并列 exact Router write set，再创建 owner |

`runtime/scheduler/src/resource.rs` 内的 ResourceTable/entry/private lease 是中央资源状态机，唯一 write owner 为
K5。A5/C5生产 source semantics；V5R只运输 facts并构造 atomic image；H5只实现 provider/composition；P5G只观察。
§2.16 的临时 overlap owner优先于 K5/H5 常规 write set，且只持续一个 atomic hard-cut checkpoint。

## 4. Proof/Gate matrix

P5G 的 expected-red commit 必须先让下表所有新场景成为真实可执行断言；不能用 label/string source scan 代替。
每扇门转绿都在同一 integration join 记录 raw command/result。完整 Gate 还必须包含 Phase 1/2/3/4 canonical
Gate regression、workspace rustfmt、workspace clippy、Gate self-tests、candidate/evidence checker。

| Gate | Contract carrier / required evidence | Producer join |
| --- | --- | --- |
| G1 / S1 source→admission | positive exact request+2 stream fixture；SSE/date.now/same-context/illegal placement无 artifact/release pointer | C5 |
| G2 / S2 admission→emission | 真实 G1 artifact 的 exact callsite/typed relocation、2 affine body takes、StreamNext/resume与 recursive drop inventory | A5+C5 |
| G3 / S3 emission→atomic-link input | 同一 artifact与 pinned registry进入 production atomic constructor并形成 exact typed targets；missing/drift/alias/swap拒绝 | V5R |
| G4 / S4 atomic-link→image | 同一 constructor闭合 bounded index/CFG/stack/slot/call/resume/schedule结构，只发布 complete image且不重建 source semantics | V5R |
| G5 / S5 image→scheduler | 同一 opaque image route：production pre-I/O Ready terminal、real TCP Parked、shared pending、2 exact handles、capacity=1 backpressure、wrong/stale ref | K5+H5 |
| G6 / S6 scheduler→request→response | 顶层 RuntimeHost exact response events；terminal/cleanup各1；pending/resource current=0、ever=true；legacy stream active=0 | H5 |
| G7 full-chain Router VCP | actual HTTP socket + Router gateway/dispatcher + actual runtime WS + RuntimeHost + outbound TCP + ordered chunked response；无 fake dispatcher/manual frame | H5+P5G |
| G8 lifecycle/race/no-blocking | A drop不影响B、duplicate drop no-op、timeout/cancel/disconnect single winner、late completion cleanup、full-buffer Pending、single-worker canary | K5+H5 |
| G9 structural fail-closed | 无 string/context/type-shape dispatch、legacy registry、second pending/root/inventory、test-only executor、hand-built image/proof bypass；无 verifier crate/API/dependency/import/selector/alias/dual path；损坏 artifact只 link error或 safe request failure且零 leak | all |
| G10 regression/quality/evidence | Phase 1–4 Gate子集、focused crates、fmt/clippy、Gate runner/checker tamper/zero/skip/stale tests | all |

## 5. Integration、validation 与 evidence epoch

- Rolling join：本 rev19 canonical/process-document closure → new clean-HEAD independent Design PASS → V5R hard cut + P5G selector companion（同一 atomic
  checkpoint）→ K5 rebase/resume → H5 rebase/resume → P5G final。A5/C5 已产生的 facts继续作为输入；若 V5R 发现
  source-semantic fact缺口则退回对应 owner并先 Amend MAP，禁止在 linker补。每次 join只机械合流 lane commit，
  不在 integration临时补语义；focused red立即退回原 owner。
- P5G 可先 join executable expected-red scaffold；producer 尚未 join 时新 Phase 5 matrix 必须 nonzero red，旧
  Phase 1–4 regression必须 green。旧 `4c8dec19` 的 90-command PASS 不属于 r1 evidence。
- 所有 Cargo 命令共享 `CARGO_TARGET_DIR=/Users/geek/workspace/.skiff-cargo-target` 且严格串行。原子租约目录为
  `/tmp/skiff-bcvm-p5-r1-cargo.lockdir`：`mkdir` 成功才运行，shell `trap` 在 EXIT/INT/TERM 时 `rmdir`；旧
  `/tmp/skiff-p5-cargo-lease` 是 cargo target 内容，不是锁，禁止使用或删除。禁止 `cargo clean`。
- 预计超过 30 秒的命令只运行一次并重定向到 `/tmp/skiff-bcvm-p5-r1-<lane>-<command>.log`，轮询日志/进程。
- merged preflight 全绿后 freeze exact commit/tree，新建 detached clean acceptance worktree。production/test/
  fixture/Gate/checker 任一变化都开启新 evidence epoch，旧 raw evidence/verdict 不可拼接复用。
- 独立 reviewer 只判 Contract 语义、已接受 invariant、第二 authority/fallback/fail-closed；其后由另一名全新
  只读 Acceptance owner运行完整 Gate并核对 raw evidence。PASS 后才写 `results/phase-5.md`、合 main/push。

## 6. Task envelope 与 handoff

每个 lane 的验收判据直接引用 Contract §5.1–§5.6，不复述。首个 `status_after` 是：A5 public authority API +
focused test；P5G executable expected-red attempt；其它 lane 为第一个真实 producer→consumer test。写集外需求、
Router defect或 shared authority选择立即 partial handoff，不能先改代码。所有 lane 按
`{完成了什么, 意外点, 尝试过什么, 需要什么}` 上报，并附 input/output commit、实际 write set、focused commands/
log、未运行项与 remaining risk。
