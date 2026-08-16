# Phase 6 Execution Map：cross-owner execution

> Status: active; production dispatch authorized from accepted Phase 5 main baseline
> `094215c624712c257aa9455fc499cc6fb3657a9e` / `ec44479d88aca83f94038f84cf8a9c38f3693ba8`
>
> Contract: [`phase-6-cross-owner-execution.md`](../phases/phase-6-cross-owner-execution.md)
>
> Activation input: exact clean accepted Phase 5 commit/tree, recorded dynamically by A0

本MAP是Phase 6当前文件写集、owner、DAG、Gate workload和join顺序的唯一权威；写集是派发时最佳已知边界，不是
不可变文件锁。P6D owner提交A0 activation record，integration owner机械合入后再按本文创建worktree。

## A0. Activation record

| Field | Recorded value |
| --- | --- |
| Phase 5 result | `doc/implementation/bytecode-vm-convergence/results/phase-5.md`，`status = accepted` |
| Phase 5 frozen candidate | `31a33c49e2358c49858b645c5f98434e3c8c91f6` / `3b631a9020077a867f8e5956322d842c421703dc` |
| Phase 5 result merge / activation input | `094215c624712c257aa9455fc499cc6fb3657a9e` / `ec44479d88aca83f94038f84cf8a9c38f3693ba8` |
| Phase 6 docs activation | `e21092b5` / `23d3050e6b96aabd97bee6bc8684cb19cf004d74` |
| Worktree roots | `/Users/geek/workspace/skiff-bcvm-p6-{integration,gate,facts,kernel,compose}-r1` |
| Cargo lease / target | `/tmp/skiff-bcvm-p6-r1-cargo.lockdir`；`/Users/geek/workspace/.skiff-cargo-target` |
| Evidence epoch | not yet minted; first expected-red G6 baseline will mint the r1 epoch from candidate-owned hashes |

Production dispatch is authorized only after these identities are clean and reachable from the worktree branches.

## 1. 有界代码接缝盘点

这是启动所需的有限清单，不是全仓architecture/reference review。行内“owner”指Phase 6修复归属。

| Current production seam | 当前事实/债务 | Phase 6 owner |
| --- | --- | --- |
| `runtime/linker/src/bytecode/link/dispatch.rs` | service参数/结果plan及effect仍可从contract concrete type/guarantee重建；interface也有重建路径 | F6删除推导，只exact join compiler facts |
| `runtime/linked-bytecode/src/targets/{interface,callback}.rs`、`candidate/validation/callbacks.rs` | target形态已存在，但完整owner/lifetime/materialization事实尚未成为所有consumer唯一输入 | F6事实/image；I6C/C6提供capability需求 |
| `runtime/scheduler/src/bytecode.rs` | run loop、unit和child executor只接一组`&mut dyn VmHeap`与budget | K6改为owner bundle/heap domain/shared request authorities |
| `runtime/scheduler/src/trampoline.rs` | blocked child主要保存unit/resume/lease，未拥有完整child heap/root/image/boundary staging | K6扩展Phase 4唯一root graph |
| `runtime/vm/src/control.rs` | `ChildInvocation`携带caller `VmOwnedValues`，不足以证明cross-heap transaction | K6定义owned invocation；F6提供exact plans |
| `runtime/request/src/bytecode_ingress.rs` | 除`StreamNext`外child统一`UnsupportedChild`；唯一request composition seam尚未分capability | X6建立唯一multiplexer；未accepted lane继续拒绝 |
| `runtime/boundary/src/service_linkable.rs`、`service_value_plan.rs` | 现有materializer端口是legacy `RuntimeValue`/`RequestHeap`，不是`ValueSlot`/`VmHeap` proof | X6实现generic VM materialization，不做adapter假通过 |
| `runtime/model/src/vm_heap.rs`、`runtime/request/src/vm_heap.rs` | checked VM heap已存在，但当前request只保留一个heap；无aggregate child memory ledger | K6增加domain/epoch、per-request MemoryLedger与exact release |
| `runtime/scheduler/src/owner_inventory.rs` | Phase 4 pending/resource/child count authority已存在，不能另建registry | K6原位扩展child heap/boundary/Actor invocation观察 |
| `runtime/boundary/src/{recoverable,persistent,db,payload}.rs` | recoverable/DB主要消费`RuntimeValue`/`RequestHeap`；VM logical snapshot seam未闭合 | D6R，在F6/K6/X6 contract之上接入 |
| `runtime/native/src/dispatch/{task,db,actor}.rs` | 旧native dispatch与新bytecode owner/image/heap的组合未证明 | T6/D6R/A6各自leaf，X6拥有共同mux |
| `runtime/host/src/host/router_session/task_submit.rs`、`router/src/task/**` | durable task control plane已存在，但payload→fresh exact-image request VCP未闭合 | T6；D6R提供codec，X6提供request seam |
| `runtime/capability-context/src/actor*.rs`、`router/src/actor/**`、`runtime/transport/src/actor*.rs` | Actor control/lease存在；VM shared arena、segment lifecycle和exact image join未证明 | A6；K6/X6处理中央kernel/mux |
| `runtime/model/src/callback_projection.rs`、`runtime/native/src/callback_adapter.rs` | same-Runtime callback已有投影/adapter；owner/lifetime/cancel与VM path未闭合 | C6；cross-Runtime明确不实现 |

启动后如果真实代码已因Phase 5变化而移动，owner先以symbol定位并在A0 amendment记录新exact path；语义归属不随
文件移动改变。发现清单之外的production seam必须先改MAP，不能顺手扩大。

## 2. Worktree、角色和长期中央owner

建议worktree直接位于`/Users/geek/workspace`：

```text
skiff-bcvm-p6-integration-r1       # I6，唯一rolling integration
skiff-bcvm-p6-gate-r1              # G6，proof/Gate only
skiff-bcvm-p6-facts-r1             # F6，compiler facts + image authority
skiff-bcvm-p6-kernel-r1            # K6，owner/heap/root/budget/memory kernel
skiff-bcvm-p6-compose-r1            # X6，boundary/request/host composition
skiff-bcvm-p6-interface-r1          # I6C
skiff-bcvm-p6-callback-r1           # C6
skiff-bcvm-p6-data-r1               # D6R，recoverable + DB
skiff-bcvm-p6-task-r1               # T6
skiff-bcvm-p6-actor-r1              # A6
```

实际agent数可以少于worktree数；同一agent可在clean closeout后串行接新lane。三个中央owner贯穿Phase：

- F6是bytecode artifact/emission/linked-image facts的中央owner；capability owner通常只提交fact requirement，
  不做F6文件写；
- K6是scheduler/VM/owner/root/budget/memory状态机的中央owner；capability owner通常只提交kernel requirement，
  不做K6文件写；
- X6是generic VM boundary、request child/control mux与host composition的中央owner；capability owner通常只实现
  下表leaf文件。

上述“通常”不是不可变文件锁：实现中为实际 seam 必需的小跨 owner 写允许先完成，在 handoff 的 actual write
set 中明确列出，由 integrator 核对记录，下一次 MAP amendment 反映 ownership 调整。

I6 integrator只cherry-pick/rebase/机械核对和运行join；它不解决production冲突、不补测试assertion。G6不改
production。需要共享文件变化时通常返回其owner；实现中发现必要的小跨owner写按§3.3在handoff上报，由下一MAP
amendment收编后，owner在自己的worktree重放最新integration并交新commit。

## 3. 精确写集

`**`只覆盖表中明确目录。新增Cargo dependency、未列`Cargo.toml`、root workspace、其它selector或其它architecture
文件都不在默认写集；需要时先Amend，或按§3.3在handoff上报实际 seam 后收编。每个文件按当前MAP任一时刻只有一个
owner；owner调整以integrator核对的actual write set + 下一次MAP amendment为准。

### 3.1 Coordination、proof与中央lanes

| Lane/status | 唯一write set | Depends / delivery |
| --- | --- | --- |
| P6D docs / active | 本Contract；本MAP；active `README.md`、`runbook.md`、`large-change-execution-principles.md`；activation后本MAP的amendments；双PASS后新建`results/phase-6.md`并做status-only closeout | planning/A0/closeout唯一docs owner；不写production |
| I6 integration / blocked | `∅`（只cherry-pick、核对、freeze和记录外部evidence，不直接编辑） | Phase5 accepted；不写production/test/Gate/docs；所需修改退回对应owner |
| G6 proof/Gate / blocked | `runtime/host/tests/bytecode_vm_phase_6.rs`；`runtime/host/tests/bytecode_vm_phase_6/**`；`runtime/host/tests/fixtures/bytecode-vm-phase-6/**`；`router/tests/bytecode_vm_phase_6.rs`；`router/tests/bytecode_vm_phase_6/**`；`scripts/lib/bytecode-vm-phase-6-{contract,gate-runner,evidence-root,evidence,receipts}.mjs`；`scripts/run-bytecode-vm-phase-6-gate.mjs`；`scripts/tests/bytecode-vm-phase-6-*.test.mjs`；`scripts/lib/verify-cli.mjs`；`scripts/lib/verify-plan.mjs`；`scripts/lib/verify-selector-graph.mjs` | A0；首个delivery是完整expected-red；production/observation schema只提需求 |
| F6 facts/image / blocked | `artifact-model/src/bytecode.rs`；`artifact-model/src/bytecode/**`；`artifact-identity/src/bytecode.rs`；`artifact-identity/src/bytecode/**`；`compiler/emission/src/bytecode/**`；`compiler/source/src/callable_effects/**`；`compiler/compiled/src/bytecode_handoff.rs`；`compiler/compiled/src/bytecode_handoff/**`；`compiler/driver/pipeline/bytecode_lane.rs`；`compiler/driver/pipeline/bytecode_lane/**`；`compiler/driver/authoring.rs`；`compiler/driver/generated_deployment.rs`；`deployment/src/projection/**`；`runtime/loader/src/deployment_bytecode.rs`；`runtime/loader/src/deployment_bytecode/**`；`runtime/linked-bytecode/src/**`；`runtime/linker/src/bytecode.rs`；`runtime/linker/src/bytecode/**` | A0+G0；长期唯一facts/image owner；先交generic boundary fact schema，再按capability request扩展 |
| K6 kernel / blocked | `runtime/model/src/{vm_heap.rs,vm_value.rs,vm_root.rs,bytecode_execution_observation.rs,service_error.rs}`；相邻同名test子文件；新`runtime/model/src/{memory_ledger.rs,actor_vm_arena.rs}`；`runtime/model/src/lib.rs`；`runtime/scheduler/src/{bytecode.rs,trampoline.rs,owner_inventory.rs,pending.rs,root_escrow.rs,lib.rs}`；`runtime/scheduler/tests/bytecode_scheduler.rs`；`runtime/vm/src/**`；`runtime/vm/tests/**`；`runtime/request/src/{execution_budget.rs,execution_control.rs,vm_heap.rs}`；相邻同名test子文件；新`runtime/request/src/memory_ledger.rs` | A0+G0；长期唯一child owner/heap/root/budget/memory/Actor arena状态机owner |
| X6 composition/service / blocked | `runtime/boundary/src/{service_linkable.rs,service_linkable_detached.rs,service_linkable_schema.rs,service_value_plan.rs,package_schema_records.rs,lib.rs}`；`runtime/boundary/src/service_value_plan/**`；`runtime/boundary/Cargo.toml`；新`runtime/boundary/src/vm_materialize.rs`与`vm_materialize/**`；`runtime/request/src/{bytecode_ingress.rs,outbound.rs,lib.rs}`；`runtime/request/Cargo.toml`；`Cargo.lock`；新`runtime/request/src/bytecode_children/mod.rs`与`bytecode_children/service.rs`；`runtime/request/tests/{bytecode_request.rs,bytecode_service_child.rs}`；`runtime/request/src/bytecode_ingress/absent_supervisor_tests.rs`；`runtime/host/Cargo.toml`；`runtime/host/src/loader/bytecode_admission.rs`；`runtime/host/src/host/{bytecode_capability_adapter.rs,bytecode_execution_observation.rs,request_supervisor.rs,runtime_host.rs,request_entry.rs,request_entry/assembly.rs,request_entry/assembly_wire.rs,request_entry/phase_2_proof_support/request_composition.rs,request_entry/phase_3_proof_support/request_composition.rs,request_entry/phase_4_proof_support/request_composition.rs,websocket_jsonrpc.rs,router_session.rs,control_plane.rs,mod.rs}` | F6+K6 J0；service first；长期唯一request/host mux owner |

F6/K6/X6的`lib.rs`/`mod.rs`通常仅由该owner接module注册；其它lane新增leaf后通常提交一行注册需求。真实 seam
需要小跨owner写这些shared files时，按§3.3在handoff上报并由下一MAP amendment收编，不静默自行编辑。

### 3.2 Capability leaf lanes

| Lane/status | 唯一write set | Central requests / depends |
| --- | --- | --- |
| I6C interface / blocked | `compiler/source/src/contract_type_resolution/interfaces.rs`及`interfaces/**`；`compiler/source/src/semantic/interface.rs`及`interface/**`；`compiler/core/src/package_interface_methods.rs`及同名tests；`compiler/lowering/src/interface_declaration_lowering.rs`；`compiler/lowering/src/source_file_lowering/tests/interface_execution.rs`及子目录；`compiler/compiled/src/projection_input/local_interface_conformances.rs`；`compiler/projection/src/package_artifact/export_links/public_instances/interfaces.rs`；新`runtime/request/src/bytecode_children/interface.rs` | J1；F6负责method/remote table facts；K6负责checked target/child semantics；X6注册leaf。I6L先，I6R后 |
| C6 callback / blocked | `runtime/model/src/callback_projection.rs`及同名tests；`runtime/native/src/callback_adapter.rs`及同名tests；新`runtime/request/src/bytecode_children/callback.rs`；新`runtime/host/src/capability_context/bytecode_callback.rs`及同名tests | service+I6L；F6负责callback facts/image；K6负责owner/root；X6注册leaf；Router reverse transport排除 |
| D6R recoverable+DB write owner / blocked | `artifact-model/src/{recoverable.rs,service_db.rs}`；`artifact-model/src/file_ir/db_indexes.rs`；`compiler/core/src/db_projection.rs`及tests；`compiler/lowering/src/db_lowering.rs`；`compiler/source/src/expression_type_model/{db_projection.rs,db_typing.rs}`；`compiler/source/src/semantic/db_attachment.rs`；`runtime/model/src/recoverable.rs`及tests；`runtime/boundary/src/{recoverable.rs,persistent.rs,db.rs,payload.rs}`及同名tests；`runtime/capability-context/src/db.rs`及`db/**`；`runtime/native/src/dispatch/db.rs`；`runtime/service-db/src/**`；新`runtime/request/src/bytecode_children/db.rs` | J1；同一owner先交R6 recoverable，再交D6 DB/reentry；F6负责bytecode expected plans/Db target closure；K6负责logical snapshot/ledger；X6注册leaf |
| T6 durable task / blocked | `compiler/lowering/src/task_call.rs`；`runtime/native/src/dispatch/task.rs`；`runtime/host/src/host/router_session/task_submit.rs`；`runtime/transport/src/protocol/task.rs`及`protocol/task/**`；`runtime/transport/testdata/task-wire/**`；`router/src/session/task.rs`；`router/src/task/**`；`router/tests/{task_control_unit.rs,task_repair_direction.rs,task_telemetry.rs,task_actor_method_execution.rs,durable_task_e2e_live_probe.rs,w_model_task_consumer.rs}`；新`runtime/request/src/bytecode_children/task.rs` | T6F依赖R6+D6+service；T6A依赖T6F+A6。F6负责task facts；K6负责fresh request observation；X6负责outbound/mux注册 |
| A6 Actor / blocked | `artifact-model/src/actor_declaration.rs`及tests；`artifact-identity/src/actor.rs`及tests；`compiler/input-model/src/actors.rs`及tests；`compiler/lowering/src/{actor_method_validation.rs,mir/builder/actor_authority.rs}`；`compiler/projection/src/package_artifact/actor.rs`；`compiler/projection/src/package_artifact/callables/implementation_manifests/actors.rs`；`runtime/capability-context/src/{actor.rs,actor_invocation.rs}`及tests；`runtime/request-contract/src/{actor_invocation.rs,actor_ref.rs}`；`runtime/native/src/dispatch/actor.rs`及tests；`runtime/host/src/capability_context/{actor.rs,actor_method_outbound.rs}`及tests；`runtime/host/src/host/actor_method_handoff/tests.rs`；`runtime/transport/src/{actor_lifecycle.rs,actor_lifecycle/**,actor_method.rs,actor_method/**,actor_owner.rs,actor_owner/**,protocol/actor.rs}`；`router/src/actor/**`；`router/src/artifact/actor_routing.rs`；`router/src/supervisor/{actor.rs,actor_sink.rs}`；`router/tests/actor*.rs`；`router/tests/actor_support/**`；新`runtime/request/src/bytecode_children/actor.rs` | service+R6+D6+J0；F6负责actor bytecode facts/image；K6负责arena/segment/root/memory central changes；X6注册leaf；T6拥有`router/src/task/actor_*` |

Amendment r1（2026-08-15）：X6 seam prep确认`BytecodeRequestExecutionInput`生产构造位于
`runtime/host/src/host/request_entry/assembly.rs`与`assembly_wire.rs`，X6 必须在该处注入
`RequestMemoryLedger`、child heap factory 与 service resolver；`runtime/boundary/Cargo.toml`与
`runtime/request/Cargo.toml`允许为`vm_materialize`/service child 增加依赖。本 amendment 只扩展 X6 写集，
不改变语义 authority 或其它 lane。

Amendment r2（2026-08-15）：K6 scheduler integration修改`BytecodeChildExecutor`签名后，唯一跨写集机械适配是
`runtime/request/src/bytecode_ingress.rs`中现有 `UnsupportedChild` port 及构造点；K6 只允许为编译兼容做机械
修改，不实现 service child mux、不改变请求语义。X6 仍拥有该文件的 service child 语义 owner。

Amendment r3（2026-08-15）：X6 给 `BytecodeRequestExecutionInput` 增加 mandatory 字段后，旧 host proof
composition callsite 与 `websocket_jsonrpc.rs` 需要机械补字段初始化；这些文件只允许由 X6 做机械编译兼容，
不改变旧 fixture/assertion/proof语义。`Cargo.lock` 随新增 dependency 由 X6 机械更新。

Amendment r4（2026-08-15）：X6 service child focused tests 使用独立
`runtime/request/tests/bytecode_service_child.rs`；`runtime/host/Cargo.toml`允许为 host-owned service
resolver/child factory 增加依赖。两者均为 X6 写集，不改变其它 lane 的语义 owner。

Amendment r5（2026-08-15）：Phase 6 增加 cross-owner boundary materializer 后，runtime crate DAG 允许
`skiff-runtime-boundary` 依赖 `skiff-runtime-linked-bytecode`/`skiff-runtime-linker`，并允许
`skiff-runtime-request` 依赖 `skiff-runtime-deployment-image`；`scripts/check-runtime-crate-dag.mjs` 由
P6D/G6 机械更新，不改变执行 authority。F6 的 service boundary plan coverage 移到 function admission 前，
保证 missing-plan 在最早门 fail closed。

Amendment r6（2026-08-15）：service lane first join 需要 source effects 对 `ContractOperation` 记
`PendingEffectCategory::ServiceCall`，并允许 Phase 1 admission 识别 service call target/category；
`compiler/source/src/callable_effects/**` 因此并入 F6 写集。G6 service fixture 改为 `typedJson` unary，
避免 raw HTTP record/stream shape 挡在 service child 语义前。

Amendment r7（2026-08-15）：service fallback closure 与 host response 并行修复。F6 额外持有
`compiler/driver/authoring.rs`、`compiler/driver/generated_deployment.rs`、`deployment/src/projection/**`、
`runtime/loader/src/deployment_bytecode.rs`及`deployment_bytecode/**`，用于确保 compiler-owned
`std.service.InternalError` fallback fact 进入 exact deployment closure。K6 额外持有
`runtime/model/src/service_error.rs`与`runtime/vm/src/**`，用于 cross-image `VmOwnedException` mint。X6 继续
持有 `runtime/request`、`runtime/boundary`、`runtime/host` 的 service response 路径；三者 write set 互不重叠。

Amendment r8（2026-08-15）：记录 J2 收敛时已合入的 ownership 调整。K6 拥有 request child mux 中的 linked
DB intrinsic child dispatch seam；D6R 拥有 in-memory serviceDb provider，并可为 exercise 它做 minimal
host-harness wiring。两者是 implementation 中发现的必要跨 owner 写，现已在 MAP 中反映为正式 ownership。

Amendment r9（2026-08-16）：记录 J3 收敛时发现的 actual ownership/seam adjustments，以下归属已正式反映到
MAP：

- K6 owns the linked DB intrinsic child dispatch seam in the request child mux（`runtime/request/src/bytecode_ingress.rs`、`bytecode_children/mod.rs`、`db_intrinsic.rs`）。
- D6R owns the in-memory serviceDb harness provider and may do minimal host-harness wiring needed to exercise it。
- A6 owns the Actor `std.actor.get` executor and the related compiler/lowering/emission/linker seams needed for exact Actor self/root facts。
- C6 owns the VM callback carrier projection and host callback hooks/resolver seam。
- I6R owns real remote interface source lowering and provider public-instance callable ABI facts；F6 owns remote provider public-instance admission/linking and composite frozen constant graph support。
- T6A owns Actor-method task admission/fence/snapshot restoration；F6 owns linked task payload plans；K6 owns task dispatch timing；X6 owns task child submission and activation identity injection。
- G6 owns containment fixtures, task harness TaskStore response, and any gate expected-count alignment。

Amendment r8 与 §3.3 provisional write-set policy 保持原样。

### 3.3 写集是 provisional boundary，不是文件锁

写集是派发时的最佳已知分解边界；dispatch-time reasoning 按设计不完整，因此实现中可能发现必要的小跨 owner
写。允许的条件是：修改对实际 seam 是必要的；在 task handoff 的 actual write set 中明确列出；integrator
核对并记录；下一次 MAP amendment 反映 ownership 调整。integrator 不把这类写自动当 violation，也不在冲突里
发明行为。

硬约束不变：同一 worktree 不并发写；proof line 不修改 production 制造 PASS；每个中央状态机任一时刻只有一个
write authority，该 authority 可在真实收敛后经 MAP amendment 调整。大范围或非必要跨 owner 写仍按写集外需求
先上报，不能借“必要 seam”静默扩大。例如没有 amendment 时，A6 不应把 `runtime/model/src/lib.rs` 当成常规写面；
若真实 seam 要求小跨 owner 写，就按本节上报并收编。

## 4. Subphase DAG、并行波次与join

### 4.1 DAG

```text
A0 activation
  |-> G0 Gate contract/self-tests/all fixtures expected-red
  |-> F6.1 generic facts/image
  \-> K6.1 owner/heap/root/memory
G0 + F6.1 + K6.1 -> J0 foundation
J0 -> X6.1 generic materializer + service -> J1
J1 -> I6L local interface -----------\
J1 -> R6 recoverable -> D6 DB ---------+-> J2
J2 + service -> I6R remote interface --\
J2 + I6L -> C6 same-runtime callback ---+-> J3
J2 + service -> T6F function task -------+
J2 + service -> A6 Actor ----------------/
T6F + A6 -> T6A Actor-method task
J3 + T6A -> J4
J4 -> full transitive preflight -> freeze -> (review cohort || detached Acceptance)
```

不存在DAG边`T6A -> A6`或`Phase7 -> Phase6`。M6 GC/compaction不在active DAG；只有Contract amendment才增加，
且必须依赖J4完整root proof。

### 4.2 并行方式

- G0与A0后的F6/K6可以同时工作；G0不得等production完成才写fixture；
- J1后I6L与D6R owner的R6→D6串行子lane在不同write set并行；
- J2后I6R、C6、T6F、A6 leaf可并行；中央F6/K6/X6 request按ready frontier批量处理，一次合入一组无冲突
  requirements，避免一个capability一个round trip；
- T6A只等T6F/A6公开seam，不等待不相关I6R/C6尾项即可开始leaf expected-red；final J4仍要求所有required lane；
- Cargo命令不并发。唯一lease为`/tmp/skiff-bcvm-p6-r1-cargo.lockdir`，共享target为
  `/Users/geek/workspace/.skiff-cargo-target`；Node/static/read-only工作可并行；禁止`cargo clean`；
- 超过30秒的命令只启动一次并重定向`/tmp/skiff-bcvm-p6-<lane>-<command>.log`，后续轮询进程/日志。

### 4.3 Join顺序与failure routing

每个join固定：leaf owner基于最新integration rebase/cherry-pick → focused check → clean commit → integrator核对
actual write set → cherry-pick → Gate workload全量继续跑 → 更新ready frontier。Integrator不在冲突里发明行为。

失败回原owner：artifact fact/atomic image回F6；owner/heap/root/budget/memory回K6；materializer/request/host mux回X6；
capability leaf回I6C/C6/D6R/T6/A6；fixture/assertion/selector/evidence回G6。若一次failure同时跨两owner，先由发现者
写最小reproducer和两个接口需求，中央owner按F6→K6→X6顺序join；不能让integrator成为临时第四authority。

## 5. Gate workload specification

### 5.1 Canonical API与selector

G6实现且只实现一套：

```js
phase6ScenarioSpecs(root)
phase6WorkloadSpecs(root)       // 本Phase + phase5WorkloadSpecs(root)，唯一transitive API
phase6CandidateSpecs(root)
phase6WorkloadProvenance(root)  // explicit P1..P6 source/parent/origin chain; no id parsing
phase6BoundedWorkLedger(root)   // P1..P6 bounded-work obligation -> transitive spec ids
assertPhase6LaneCoverage(specs)
assertPhase6ProvenanceCoverage(specs, provenance)
assertPhase6BoundedWorkCoverage(specs, ledger)
```

公开selector为`bytecode-vm-phase-6-gate`。Runner命令与环境遵守Contract §6；selector expansion只指Phase 6
runner，不嵌套Phase 1–5 selectors。`phase6WorkloadSpecs(root)`复制/re-id inherited entry时保留
`cwd/command/testFormat/lanes/expectedTests`，记录immediate `parentPhase/parentId`并追加`phase-5-regression` lane；args只允许
对`cargo test`在`test`后幂等插入一次`--no-fail-fast`，其它target/filter/harness args逐字不变，build/fmt/clippy
不插入。不能从旧receipt生成spec。

本Phasespec的稳定字段：`id`、`cwd`、`command`、immutable `args`、`testFormat`、immutable `lanes`、
`expectedTests`、`sourcePhase/sourceId`、`parentPhase/parentId`、immutable `originChain`。本Phase所有
`testFormat != null` workload的`expectedTests`为正整数；
Gate self-test证明Node/Rust实际数多一个或少一个都会FAIL。Inherited entries的`expectedTests`字段缺失与显式
`null`必须原样保留，由既有parser做`>0/no skip`（`rust-exact`另保证1），并在Phase6 result逐entry列出，不能静默
补`1`。Planning snapshot中Phase 1–4普遍缺字段，Phase 5 exact entries也缺字段，另有
`k5-scheduler-resource-authority`与`k5-capacity-one-stream-lifecycle`显式`null`；A0按accepted API重新生成精确清单。
本Phaseown spec固定`sourcePhase = 6`、`sourceId = id`、无parent、`originChain = [{phase: 6, id}]`。
Inherited provenance由G6在candidate-owned显式catalog逐entry列出真实source和每次累计composition id；不得通过
`phase-N-regression-`前缀解析。Coverage assertion要求provenance与最终workload双射、chain phase严格递增、末端为
final Phase 6 id，且sourcePhase 1–6均非空。

`phase6BoundedWorkLedger(root)`使用稳定obligation键并只引用`phase6WorkloadSpecs(root)`中的id：

| Obligation key | Canonical owner/evidence |
| --- | --- |
| `p1-dispatch-fuel` | Phase 1 dispatch/raw-fuel specs |
| `p2-p3-cleanup-unwind` | Phase 2 lifecycle/drop与Phase 3 unwind/cleanup specs |
| `p4-wake-claim` | Phase 4 publish/wake/claim/terminal-race specs |
| `p5-stream-pump-buffer` | Phase 5 stream pump/backpressure/buffer cleanup specs |
| `p6-materialization-root-walk` | 本Phaseservice/data/kernel materialization、root walk、memory release specs |

Coverage assertion拒绝空数组、未知/重复spec id、漏obligation或把quality/candidate command冒充bounded-work证据。
若继承矩阵无法提供某项，先reopen其owner Phase补canonical spec并重建当前epoch；Phase 7只消费/聚合，不补规则。

### 5.2 First-day executable catalog

G0必须一次创建所有fixture、test skeleton、spec和checker。Skeleton必须调用production入口并在尚未实现处做真实
assertion；不得`ignore`/`todo`/conditional skip。下面每个Rust prefix是一条独立spec，使用
`cargo test --no-fail-fast ... <prefix>`并以`expectedTests`固定命中数量；Rust harness在prefix内继续运行所有test，
outer runner在该spec失败后继续下一个spec。

| Spec id/prefix | Test binary | Cases必须至少包含 | Lanes |
| --- | --- | --- | --- |
| `p6-service-matrix` / `service_` | host `bytecode_vm_phase_6` | S1–S6、sync、Pending、throw、partial allocation、cancel/deadline/late/duplicate | S6,F6,K6,X6 |
| `p6-interface-local-matrix` / `interface_local_` | host | S1–S6、local table success/throw/Pending、bad slot/carrier/signature | I6L,F6,K6 |
| `p6-interface-remote-matrix` / `interface_remote_` | host | S1–S6、remote unary/stream/throw、protocol/build drift | I6R,S6,F6,X6 |
| `p6-callback-matrix` / `callback_` | host | S1–S6、unary/stream lifetime、Pending/cancel/expired/cross-runtime reject | C6,F6,K6,X6 |
| `p6-recoverable-matrix` / `recoverable_` | host | S1–S6、plain/envelope/local behavior roundtrip、bad/ambiguous/partial decode、cross-service reject | R6,F6,K6 |
| `p6-db-matrix` / `db_` | host | S1–S6、read/write、commit/abort、nested/reentry、Actor DB-only、Pending cleanup | D6,F6,K6 |
| `p6-task-host-matrix` / `task_` | host | S1–S6、submit/fresh attempt/exact build/payload/parent independence | T6F,R6,D6,X6 |
| `p6-task-router-matrix` / `task_` | router `bytecode_vm_phase_6` | TaskStore accept/claim/lease/fence/retry/late/duplicate | T6F,T6A |
| `p6-actor-host-matrix` / `actor_` | host | S1–S6、create/sync/Pending/reacquire/arena cap/DB-only transaction | A6,R6,D6,K6,X6 |
| `p6-actor-router-matrix` / `actor_` | router | exact owner/build/fence/idle/discard/cross-build + Actor task | A6,T6A |
| `p6-containment-matrix` / `containment_` | host | cross-runtime callback、cross-service behavior envelope、GC/concurrent/verifier surfaces fail closed | NEG,F6,X6 |
| `p6-kernel-focused` / `phase_6_` | scheduler/request/VM package tests | owner bundle, root visit, memory reserve/release, sync-no-park, actual-Pending chain | K6 |
| `p6-no-verifier-structural` | Node checker | crate/API/import/selector/alias/seal/dual path为零；linker mutation不能重建facts | NEG,F6,G6 |
| `p6-gate-self-tests` | Node TAP | zero/skip/stale/tamper/path escape/symlink/expectedTests/selector/no-fail-fast/lease/provenance bijection | G6 |
| `p6-fmt`、`p6-clippy` | Cargo | `fmt --all -- --check`；workspace clippy policy | QUALITY |
| `phase-5-regression-*` | inherited specs | Phase 5完整transitive workload，不调用旧Gate | REGRESSION |

每个capability fixture都有positive和negative源目录，且同一fixture identity从S1流到S6。Host/Router测试可以用
deterministic in-process provider/TaskStore/DB/clock，但compiler、artifact publication、atomic constructor、request
entry和实际consumer不可替换。

### 5.3 Required lane coverage

`PHASE6_REQUIRED_LANES`至少包含：

```text
G6 F6 K6 X6 S6 I6L I6R C6 R6 D6 T6F A6 T6A
SENTINEL NEG RACE MEMORY ROOT CLEANUP BOUNDED-WORK QUALITY phase-5-regression
```

每个lane必须由至少一个本Phasespec承载；`SENTINEL`还要机械证明每个accepted capability都出现S1–S6六个case。
Candidate specs分别在preflight/postflight/closure/fresh捕获HEAD/tree/status。Gate verdict同时要求所有workload receipt
存在，即使早期workload nonzero；只有SIGINT/SIGTERM允许中止，且中止verdict固定FAIL并列出未执行spec。

### 5.4 Error与observation schema

G6不按stderr字符串断言。F6/K6/X6分别提供typed observation：artifact/image rejection category；owner transition/
heap domain/root count/budget/memory；request/host terminal与cleanup。Capability owner提供自己的typed outcome。Manifest
至少聚合：

```text
source/artifact/image identity
schema/ISA read from candidate
capability + S1..S6
caller/provider image and heap-domain identities
owner/pending/root/resource/staging counts before, peak, terminal
fuel and memory before/charged/released/terminal reason
completion winner/resume count/late-drop count
capability-specific build/lease/fence/transaction/task outcome
```

业务值只记录fixture-defined digest，不记录秘密或任意payload。

## 6. Capability completion contracts

以下是agent任务可引用的最小完成条件；详细语义只引用Contract，任务信封不得复制改写。

| Checkpoint | Required Gate evidence | Unlocks |
| --- | --- | --- |
| J0 foundation | F6/K6 unit + service expected-red已到S4/S5；no verifier/no reconstruction；owner bundle/memory ledger接口冻结 | X6 service |
| J1 service | service S1–S6/full-chain/negative/race全绿；different owner+heap；sync no park；Pending root chain；cleanup zero | I6L,R6 |
| I6L | local interface exact method table/carrier + checked dispatch，S1–S6全绿 | I6R,C6 |
| R6 | owner-internal recoverable S1–S6；restore identity、partial decode、unsupported carrier fail closed | D6 |
| D6 | DB/transaction S1–S6；exact target、nested/reentry、Pending cleanup、Actor field-write fail closed | T6F,A6 |
| I6R | remote table→service child exact operation/build；unsupported remote shape拒绝 | capability ledger entry |
| C6 | same-Runtime callback owner/lifetime/cancel；cross-Runtime negative | capability ledger entry |
| T6F | durable accepted record→fresh request；exact build/payload；lease/fence；parent independent | T6A |
| A6 | exact build/image/implementation；shared arena/segment actual-Pending；DB-only transaction；arena cap/discard | T6A |
| T6A | TaskStore→Actor get-or-activate/method；task和Actor双fence/at-least-once | J4 |
| J4 | 全ledger、transitive Gate、memory/root/cleanup、disabled negatives全绿 | freeze |

## 7. Task envelope规则

每个写任务必须包含且由派发者机械核对：

1. exact input commit/tree、branch/worktree、lane与当前evidence epoch；
2. Contract精确小节和本MAP checkpoint/Gate spec引用；验收判据不得复述语义；
3. 本表exact write set子集、当前排除其它lane（尤其F6/K6/X6/G6与I6 integration）的文件；若实现中发现必要
   跨owner写，按§3.3在actual write set中显式列出，不静默扩大；
4. depends-on commit与首个可观察`status_after`（expected-red/partial/ready-for-join）；
5. focused command、Cargo lease、唯一`/tmp`日志路径、未运行项；
6. production seam需要中央owner时，提交`{required fact/API, caller, consumer, failing spec}`，不得附第二实现；
7. 上报`{完成了什么, 意外点, 尝试过什么, 需要什么}`、output commit/tree、actual write set、status clean。

必要的小跨 owner 写允许随任务先完成，并在 actual write set 中上报；其余写集外需求立即 partial handoff。MAP
amendment 先合入 integration 并广播新 tree，之后 owner 继续按新 ownership 工作。任务完成后尽快提交；不把
dirty worktree 当依赖。

## 8. Freeze、Acceptance与资产结案

J4 merged preflight全绿后：

1. integration owner记录exact commit/tree/status与production compiler artifact/schema/ISA/image constructor；
2. freeze后不再cherry-pick；任何production/test/fixture/Gate/observation/schema变化开启新epoch并重新freeze；
3. 同时启动全新只读semantic review cohort：F6 facts/image authority、K6/X6 ownership/memory、G6 proof/false-green
   三个互斥主题读同一commit/tree；不审architecture文档完备性；
4. 另一名全新Acceptance owner同时从frozen commit创建detached clean worktree，运行完整canonical Gate并核对raw evidence；
5. 所有reviewer返回前不开始修复；integrator只合并/去重一个blocker list，再按§4.3一次批量返回owner。任一修复产生
   新freeze/epoch，旧review/Acceptance/receipt均不可复用；
6. 两边PASS后P6D owner只允许写`results/phase-6.md`及README/MAP status allowlist，status只能是`accepted`；
   result记录frozen candidate/evidence；integration owner机械合入该commit，并在handoff另报closeout/main exact
   commit/tree与clean status。Phase 7 A0动态记录实际baseline并证明candidate→baseline无
   production/test/fixture/Gate/schema变化。触及allowlist外文件则重新freeze/Acceptance。

Worktree/branch/evidence必须逐项结案：

- merged leaf：确认commit已可达、worktree clean后删除worktree和临时branch；
- rejected/unused leaf：先审`git status`/diff；有独立价值则提交到明确命名的salvage branch并在result写disposition，
  无价值才删除；禁止遗留未提交代码；
- integration：accepted result合入main并验证tree后删除；main checkout始终留在main；
- detached acceptance：verdict落账后删除；
- stash/archive ref：Phase 6默认不建stash；若incident产生，逐个`stash show`审计，有价值内容先固定到
  `refs/archive/bcvm-p6/<lane>/<epoch>`并记录commit/tree，无价值才精确drop。不得通配清理或触碰Phase 1–5/7资产；
- evidence root：保留immutable raw evidence到Phase 7 accepted；不移动进repo、不作为新Gate输入；过期epoch在result列
  superseded原因，之后按项目policy删除；
- Cargo lease：runner finally释放并证明目录不存在；不得留下后台Cargo/rustc或writer进程。

终态反向核对：无`skiff-bcvm-p6-*` active worktree、无`refs/heads/codex/bcvm-p6-*` active branch、无Phase 6
stash、无Cargo lease/后台writer；已在result登记的archive refs与repo外immutable evidence除外。

Phase 7只接收accepted result、`phase6WorkloadSpecs(root)`和ledger，不接收worktree、dirty diff、旧receipt或“可能
可用”的WIP。

## 9. Non-goals与操作红线

本MAP不安排cross-Runtime callback、cross-service behavior envelope、exactly-once task、DB/TaskStore分布式事务、
Actor live heap迁移、GC/compaction、`concurrent`/`serial`、性能优化或旧schema compatibility。

禁止：并发Cargo；integrator/G6改production；未经handoff上报且未经下一MAP amendment收编的跨owner写；linker
按类型重建fact；raw handle跨heap；手造image/fiber/owner制造PASS；首个失败后停止matrix；以零测试/skip作为
expected-red；nested old Gate；复用旧receipt；在merge冲突里添加fallback；未审diff直接删除dirty worktree。
