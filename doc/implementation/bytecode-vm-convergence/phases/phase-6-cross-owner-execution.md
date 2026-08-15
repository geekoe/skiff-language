# Phase 6：cross-owner execution and managed-memory readiness

> Status: active; activated from the accepted Phase 5 main baseline
> `094215c624712c257aa9455fc499cc6fb3657a9e` / `ec44479d88aca83f94038f84cf8a9c38f3693ba8`
>
> Semantic Closure: compiler-owned cross-owner facts, one atomic deployment image, one flat owner-aware trampoline,
> transactional typed materialization, capability-by-capability acceptance
>
> Depends on: Phase 5 status `accepted` at an exact clean commit/tree
>
> Unblocks: Phase 7 whole-system closure; Phase 7 consumes only the ledger accepted here

本文是 Phase 6 的 active Contract。执行 DAG、文件写集、当前代码接缝和 merge 顺序见
[`phase-6-execution-map.md`](../tasks/phase-6-execution-map.md)；公共执行流程见
[`runbook.md`](../runbook.md)。历史 [`doc/implementation/bytecode-vm/`](../../bytecode-vm/) Phase 6
文件只提供 workload 线索，不提供状态、authority 或实现顺序。

Phase 6 不设置 architecture 文档完备性 review。实现开始前只做下文的机械 activation；最终 semantic review
只审 frozen implementation/test candidate。

## 1. Activation、baseline 与版本

本计划提交不是 implementation baseline。Integration owner 只有在 Phase 5 result 明确记录
`status = accepted` 后才创建 Phase 6 production worktree；`candidate-pass`、局部 Gate PASS、旧 receipt、dirty
worktree 或仅合入一部分 Phase 5 commits 都不满足条件。

Phase 6 的第一个 activation commit 必须从实际候选动态记录：

| Field | Source |
| --- | --- |
| Phase 5 result path/status | accepted result 文件；status 必须是 `accepted` |
| input commit/tree | clean accepted Phase 5 commit及其 `HEAD^{tree}` |
| integration/main identity | Phase 5 accepted result记录的合入identity；若 activation 直接基于accepted commit，则记录两者等价关系 |
| compiler artifact identity | 用该候选的production compiler生成一份真实fixture artifact后读取，不能手填fixture identity |
| bytecode schema/ISA | 从该候选实际常量/manifest读取，不在本Contract固定数字 |
| image constructor | 该候选唯一 `DeploymentExecutionImage` atomic constructor的代码identity/入口 |
| evidence epoch | compiler/runtime/Gate/fixture/observation schema各自hash与epoch |
| Cargo lease/target | `/tmp/skiff-bcvm-p6-r1-cargo.lockdir`；`/Users/geek/workspace/.skiff-cargo-target` |

Activation 验证上述 commit/tree clean、可达且相互一致；任何版本值不可从本计划、旧 Phase 6、Phase 5 中间分支
或手写常量复制。Phase 6 若改变持久 artifact facts，直接 hard cut 到由代码定义的新 schema/ISA；Skiff 尚未
发布，不增加旧 artifact reader、compat branch 或 dual path。

Phase 5 的 `phase5WorkloadSpecs(root)` 是本 Phase transitive regression 输入；Phase 6 复用 workload specs，
不嵌套运行 Phase 5 Gate，也不信任旧 Phase 5 receipt 作为当前候选证据。

## 2. 不可协商的 authority hard cut

### 2.1 Producer、transport 与 consumer

1. source analysis/compiler 是 source semantics 的唯一 owner：boundary placement、call target、callable effect、
   `maySuspend`、carrier、owner/lifetime、parameter/result/error/stream-item plan、runtime tag、transfer/drop、
   recoverability、transaction restriction 与 source attribution均在这里决定；
2. artifact 只运输 compiler 已决定的 exact typed facts。缺失、歧义、越界或自相矛盾在 emission/admission
   fail closed；
3. decoder/pre-link validator只做不可信字节、schema/ISA、长度/索引/资源上限和局部结构检查；
4. linker只在**一个**原子 `DeploymentExecutionImage` constructor中解析 exact package/deployment/type/shape/
   function/table/plan引用，并闭合有限结构一致性。它不得按名字、nominal type、shape、registry或上下文重建
   materialization、effect、carrier、recoverability或placement；
5. scheduler、VM、request和host只消费完整image的opaque indexed view，并保留必要的checked lookup；损坏
   artifact只能得到bounded link error或安全request failure，不能越界、panic、发布半成品image或泄漏owner；
6. production与test均不存在独立verifier crate/stage/API、`Verified*` facts、seal、verifier receipt、compat shim、
   alias selector、`link -> verify`边界或“测试专用旧路径”。

Phase 5 已接受的 result/`EmitStream` facts和runtime type tags是输入。`TypeIndex(0)`、record自身tag及每个field的
exact tag都必须跨owner保持；consumer不能用site type或`TypeRef == Shape`猜回缺失fact。

### 2.2 Capability admission ledger是唯一开关

每个call target在compiler emission、artifact admission、atomic image construction和runtime dispatch使用同一个
capability identity。状态只有：

- `accepted`：本 Phase exact frozen candidate上对应VCP/Gate与独立Acceptance通过；
- `disabled`：compiler或唯一admission/dispatch入口稳定fail closed，且negative Gate证明不可达；
- `planned`：实现尚未开始，运行时必须等同`disabled`；
- `enabled-unaccepted`：只允许作为当前正在修复的短期开发态，不能freeze或交给下游。

不得保留“artifact可发出但request才偶然Unsupported”的多门漂移。尚未accepted的surface必须在最早拥有完整
判据的唯一门拒绝；runtime仍对损坏image做checked failure。

## 3. Cross-owner 执行契约

### 3.1 一个 execution owner bundle

每个root或child execution unit拥有一个不可拆分的owner bundle：

```text
ExecutionUnitOwner
  exact DeploymentExecutionImage pin
  executable/function identity + invocation identity
  owner-local heap domain/epoch
  VM unit/frame/UnwindState
  VmRootSource registration
  shared request ExecutionBudget reference
  shared request MemoryLedger reference
  owner-local resources/callbacks/boundary staging
  parent/child continuation lease (root除外)
```

Bundle在进入callee之前原子构造；缺任一字段不得把parent改成blocked。一个execution unit只有一个bundle owner，
一个heap domain只有一个lifecycle owner。`RequestExecutionOwnerInventory`是Phase 4已接受owner graph的唯一扩展点；
Phase 6给它增加child heap/boundary/Actor invocation观察，不建立第二个root registry、sidecar owner map或隐藏
pending list。

Root graph必须枚举：所有VM frames/locals/operand roots、每个blocked parent与active leaf bundle、Pending owner、
ResourceTable owner、stream/callback capture、boundary source/destination/staging roots、ordinary error/unwind payload、
cleanup continuation，以及Actor方法的request locals。Actor instance field roots属于instance arena graph，不混入
request heap graph；当前Actor invocation持有对arena/fence的显式lease root。

### 3.2 Flat child lifecycle

所有同步和Pending child使用同一个flat scheduler/trampoline，不递归进入native Rust call stack，也不建立
service/task/interface/callback/Actor各自的child loop。状态转移固定为：

```text
Prepared --atomic publish--> Running
Running --sync result/throw--> Settling --> Released
Running --actual Pending--> Blocked --wake/claim once--> Running
Running/Blocked --request stop--> Stopping --> Released
```

- `Prepared`失败：释放destination partial allocation/image pin/lease；parent仍running且未发布resume token；
- sync `Ready`：不产生Pending、不park parent；result/ordinary throw materialize完成后parent恰好resume一次；
- actual `Pending`：blocked node拥有整条parent→leaf链的unit、heap、roots、resume、resources与image pins；只保存
  opaque checked indices，不保存借用或raw pointer；
- wake、cancel、deadline、disconnect和completion竞争由Phase 4唯一claim/terminal arbiter裁决；late/duplicate
  completion只清理自己的资源，不能再次写parent；
- terminal后由同一owner graph反向释放child→parent；任何路径都不能遗留half image、root、heap、buffer、
  resource、callback capability或owner inventory entry。

### 3.3 Typed boundary materialization是事务

跨heap只传logical typed value，不传raw `ValueSlot`、heap handle、mutable root、`InOut` loan、VM frame、Actor
field root或image-local table index。compiler-emitted plan同时描述argument/result/ordinary-error/stream-item的
expected type/shape/runtime tags、carrier、transfer/drop和source attribution。

Materialization的提交顺序固定：

1. source value和source owner root保持live；
2. destination使用新heap domain按exact plan递归分配；每个checked read/allocation向MemoryLedger计费；
3. 全图及carrier/lifetime/tag校验完成；
4. destination root一次发布；
5. 只有在plan声明move时才释放source owner；snapshot/copy保持source语义；
6. 任一步失败都撤销未发布的destination staging，source仍完整且可清理。

这只是一次boundary copy transaction，不是业务transaction：它不回滚已经执行的callee effect、Actor field write
或DB write。Result/error writeback开始后request terminal获胜时，未发布结果被丢弃并清理，不能晚写caller。

### 3.4 Fuel、deadline、cancel 与 memory

- 一个request只有一个Phase 1 `ExecutionBudget`，它是raw fuel、request/root deadline、cancel/internal-stop和
  terminal winner的authority；child entry只借同一引用，不能reset、mint、grant、refund或换token；
- 每次VM step、child dispatch、boundary graph visit/allocation、codec node、resume和cleanup按既有contract计费；
  provider内部的独立外部配额可以存在，但不能替代或延长request budget；
- local lexical timeout仍由其scope owner投影可捕获错误；request/root/inherited deadline只形成request terminal；
  capability-specific primitive timeout只有在caller continuation仍active时才投影其公开错误，不能伪装成
  root timeout；
- 一个request只有一个`MemoryLedger`。它聚合所有owner-local child heaps、VM stack/frame容量、boundary staging、
  Pending/owner节点、resource/callback/stream buffer与cleanup保留；owner-local limit可以更小，但不能绕开aggregate
  hard cap；reserve/commit/release必须可观察且exactly once；
- Actor instance arena使用独立per-incarnation ledger/hard cap；Actor方法的locals、request-boundary staging和
  pending control仍计入调用request ledger。arena不得把增长记到任意caller request后遗忘。

内存超限是bounded platform/request failure；它与instruction fuel、deadline分别记录，不能以OOM/panic或不受控
allocator增长实现。

### 3.5 Error 与 failure atomicity

下列结果必须区分并保留唯一terminal：success、ordinary typed throw、catchable boundary/capability error、
uncatchable execution-budget terminal、artifact/image rejection和platform/internal failure。Callee ordinary throw按
compiler error plan materialize到caller并保持catch identity；artifact corruption不伪装成用户throw。

任何失败必须满足：未发布image不可见；未发布destination graph不可见；parent最多恢复一次；源值直到transfer
commit仍有owner；所有partial resources可枚举并释放；日志/observation不得包含未初始化业务值或底层DB/transport
秘密。

### 3.6 DB transaction 与 recoverable归属

Phase 6必须接受owner-internal recoverable codec与DB bytecode lane，因为durable task payload和真实Actor create/
method workload依赖它们。边界authority固定为：

```text
compiler recoverability/DB schema and transaction facts
  -> artifact exact expected plans/target identity
  -> atomic image exact closure
  -> checked VM boundary codec / DB capability
  -> ServiceDb store / TaskStore
```

- recoverable encode/decode消费logical typed snapshot，不消费`ValueSlot` bytes、heap address/generation、COW backing、
  const index、frame或resume id；owner-internal`LocalConcrete`只在当前exact image内按stable key恢复；
- callback capability、live resource、Pending、transaction guard、`InOut`和cross-service behavior envelope保持
  disabled；普通plain data仍按其canonical lane；
- DB operation使用exact `DbObjectTargetId`和compiler-normalized result plan，不按type name重查metadata；
- 一次execution最多一个active DB transaction。compiler拒绝静态nested/reentry；runtime在dynamic helper/reentry
  checked拒绝，不能复用或隐式嵌套session；
- transaction token由当前`ExecutionUnitOwner`唯一持有；commit、abort和adapter cleanup若actual `Pending`，必须
  经Phase 4同一Pending owner/root/claim路径park，request terminal获胜后仍执行有界cleanup再释放token，不能提前
  宣告资源归零或二次resume；cleanup失败记录DB/platform failure但不把已选择的terminal改写成用户throw；
- commit/abort只作用于DB。普通request/Actor heap、Env和已经执行的Actor writes均不回滚；Actor transaction body
  的field write由compiler effect closure拒绝，unknown/dynamic target不能证明安全时保守拒绝；
- task submission不得在active DB transaction中发生；TaskStore acceptance与service DB commit不是原子事务，
  不宣称exactly once。任务需要业务幂等或outbox时由业务数据面表达。

### 3.7 GC、compaction 与性能进入条件

Phase 6 **必须**交付统一MemoryLedger和bounded no-GC执行；request tracing/concurrent GC、moving GC、通用cycle
collector和性能调优明确defer，状态为`disabled`，不阻塞Phase 6 acceptance。

只有同时满足以下条件，才能通过Contract amendment激活可选M6 compaction实验：所有accepted capability的完整
root graph Gate通过；每种Pending/cleanup/partial materialization均有root observation；handle domain/epoch能拒绝
stale handle；hard cap仍独立生效；stress matrix能证明peak/release而非只测吞吐。未满足时不得以pin-all、扫描Rust
stack、第二root registry或隐藏global arena绕过。

Actor arena同样先以per-incarnation hard cap和whole-instance discard有界运行；quiescence compaction若未达到
`active = suspended = cleanup = 0`、field roots完整、discard优先、epoch bump原子等条件，就在final ledger中记为
`disabled/deferred`。Phase 7不能临场打开它。

## 4. Capability DAG 与目标ledger

Phase 6使用以下无环顺序。`||`表示leaf work和expected-red可并行；共享F6/K6/X6文件仍由其唯一owner串行join。

```text
A0 accepted Phase5 activation
  -> (G0 executable expected-red Gate
      || F6 facts/image
      || K6 owner/heap/root/memory kernel)
G0 + F6 + K6 -> J0 atomic foundation
J0 -> S6 same-Runtime service -> J1 service checkpoint
J1 -> (I6L local interface || R6 owner-internal recoverable -> D6 DB)
I6L + D6 -> J2 data/local-interface checkpoint
J2 -> (I6R remote interface || C6 same-Runtime callback
       || T6F durable function task || A6 Actor)
I6R + C6 + T6F + A6 -> J3 capability wave complete
T6F + A6 -> T6A durable Actor-method task
J3 + T6A -> J4 frozen Phase6 candidate
J4 -> (semantic review cohort || detached Acceptance) -> accepted result
```

目标支持面及默认退出状态：

| Surface | Activation target | Depends | 未accepted时唯一状态 |
| --- | --- | --- | --- |
| F6/K6 shared facts/kernel | required `accepted` | A0/G0 | Phase 6 blocked |
| same-Runtime service child | required `accepted` | J0 | disabled at admission/dispatch |
| local `any I` interface | required `accepted` | J1 | disabled |
| remote interface via service operation | required `accepted` | service + I6L | disabled |
| same-Runtime callback capability | required `accepted` | service + I6L | disabled |
| owner-internal recoverable codec | required `accepted` | J1 | disabled |
| DB/transaction | required `accepted` | recoverable + J1 | disabled |
| durable function task | required `accepted` | recoverable + DB + service request entry | disabled |
| Actor get/create/method | required `accepted` | service + DB + K6 | disabled |
| durable Actor-method task | required `accepted` | T6F + A6 | disabled |
| cross-Runtime callback/reverse callback | `disabled` | missing Router reverse transport | `CapabilityUnavailable` |
| cross-service behavior-bearing recoverable envelope | `disabled` | missing sealed transport contract | recoverable boundary rejection |
| request GC / Actor compaction | `disabled/deferred` | §3.7 amendment only | bounded hard cap/no-GC |
| `concurrent`/`serial` | `disabled` | outside this Phase | compile rejection |

若实现证据证明某个required capability不能在本 Phase正确落地，必须先Amend Contract、Phase 7 support target和MAP，
将它显式降为`disabled`；不能以“后面再补”接受Phase 6。

## 5. VCP 与 stage-sentinel matrix

### 5.1 六个共享sentinel

每个capability使用真实`.skiff` fixture，逐层传递production产物，并至少有六个独立test case：

| Sentinel | Production boundary与必查事实 |
| --- | --- |
| S1 source→admission | source semantics产生exact capability/boundary/recoverability/transaction facts；非法或disabled surface不产artifact/release pointer |
| S2 admission→emission | production artifact含exact target、argument/result/error/item、runtime tags、carrier、owner/lifetime、transfer/drop/source facts |
| S3 emission→atomic-link input | 真实compiler artifact、exact deployment/package closure、动态schema/ISA进入唯一constructor；missing/drift/swap拒绝 |
| S4 atomic-link→image | constructor只exact resolve并原子发布complete image；无verifier、semantic reconstruction、partial cache/image |
| S5 image→scheduler/VM | opaque image view进入对应consumer；owner/heap/budget/root/lease与sync/actual-Pending行为可观察 |
| S6 scheduler/VM→request/host/router→terminal | 真实request/host（需要时Router/TaskStore/Actor owner）产生success/error/stream terminal；owner/root/resource/heap/staging计数归零 |

S3/S4是同一constructor的输入与完成态观察，不建立第二个API。Proof可以注入deterministic clock、provider、
allocation failure、TaskStore/DB fake和completion race，但必须从production composition seam注入；禁止手造artifact、
image、method table、child fiber、owner token、heap handle、resume token或最终response。

### 5.2 每条production path的最小可执行矩阵

下表每行都必须有S1–S6和至少一个full-chain workload；`错误类别`是Gate稳定分类，activation时映射到实际Rust/public
enum，不能靠message substring。

| Lane/path | Producer → artifact fact → atomic image | Runtime consumer | Positive workloads | Negative/race workloads | Expected error category |
| --- | --- | --- | --- | --- | --- |
| S6 service | service call semantics → exact provider/argument/result/error/resume plans → pinned provider image | request child mux → flat scheduler/VM → typed materializer → host response | sync nested record/union/TypeIndex0；actual Pending；ordinary throw | wrong build/protocol；missing/swapped fact；raw cross-heap handle；partial destination allocation；cancel/deadline/late/duplicate | compile/admission rejection, image-construction rejection, typed execution failure, budget terminal |
| I6L local interface | explicit `as I`/conformance → local carrier + exact method table/slot/signature → caller image | checked interface dispatch in same owner/heap | local method success/throw；concrete sync and Pending | wrong carrier/table/slot/signature；`inout`；linker fact reconstruction trap | compile rejection, image-construction rejection, checked dispatch failure |
| I6R remote interface | remote box provenance/operation ids → exact remote table + service plans → caller/provider images | interface dispatch → service child mux → provider | remote method success/stream/throw | protocol/operation drift；remote result含unsupported `any`；provider pointer change mid-call | admission/image rejection, capability/boundary failure |
| C6 callback | compiler callback projection → owner build/route/operation/lifetime facts → caller/provider images | same-Runtime capability table → owner execution context | unary callback；stream-lifetime callback；actual Pending | expired/wrong owner/wrong operation；cancel then late callback；attempted cross-Runtime route | capability unavailable/expired, budget terminal |
| R6 recoverable | recoverability/schema facts → exact expected codec/restore plans → image | logical VM snapshot → owner-internal boundary codec → fresh heap | plain+envelope roundtrip；nested local behavior restore；strict/durable policy | malformed/stale envelope；callback/resource/`InOut` encode；ambiguous restore；partial decode graph；cross-service behavior | recoverable/compile/image rejection |
| D6 DB/transaction | DB schema/transaction facts → exact `DbObjectTargetId`/result/codec plans → image | VM DB effect → boundary/capability → ServiceDb | full/projected read/write；commit/abort；Actor DB-only transaction | nested/dynamic reentry；Actor field-write reachability；target drift；constraint；Pending cleanup race | DB transaction/constraint failure, compile/image rejection |
| T6F function task | dispatch semantics → exact target/build/recoverable payload plan → image + TaskRecord | host submit → Router/TaskStore claim/lease/fence → fresh request | durable acceptance then function attempt；retry/lease transfer；parent terminal independence | active DB transaction；bad payload/build/target；duplicate/late settlement；lease loss | task admission/recoverable rejection, fenced attempt failure |
| A6 Actor | actor/get/method facts → exact ABI/implementation/build/field/method plans → caller+owner images | Router exact owner → segment lease → shared arena VM invocation | get/create；sync method；actual Pending/reacquire；DB-only create/method | cross-build；lease/fence/epoch mismatch；request-scoped field write；cancel/idle/discard races；arena cap | actor version/admission rejection, bounded invocation/budget failure |
| T6A Actor task | actor-method dispatch + activation snapshot → task target/build/payload facts → task+Actor images | TaskStore → Actor get-or-activate → segment lease | not-live activate+invoke；live same-build invoke；at-least-once retry | live different build；stale activation plan；task fence vs Actor fence；duplicate settlement | task/actor version or fencing rejection |
| containment | unsupported source/carrier facts absent → no executable image entry | unique admission/dispatch guard | declared disabled ledger visible | cross-Runtime callback, cross-service behavior envelope, GC/compaction, concurrent/serial, verifier/API name | compile/admission/capability rejection |

矩阵并行规则：G0的JS/checker、fixture authoring与static manifest检查可以并行准备；F6与K6、J1后的I6L与R6→D6、以及后续
I6R/C6/T6F/A6的互不重叠leaf work可以并行。所有Cargo命令共享一个lease并串行；修改F6 facts、K6 trampoline/
owner graph或X6 request multiplexer的join也串行。Final Gate runner可以并行非Cargo read-only candidate checks，
但必须等待所有workload落账并在失败后继续其它workload，不能outer fail-fast。

## 6. Gate contract

G6在第一个production join前交付以下canonical assets：

- selector：`bytecode-vm-phase-6-gate`；
- command：`node scripts/run-bytecode-vm-phase-6-gate.mjs --candidate <40hex> --tree <40hex> --output-dir <absent-absolute-dir>`；
- selector环境等价为`SKIFF_BYTECODE_VM_PHASE6_CANDIDATE_COMMIT`、`..._CANDIDATE_TREE`、
  `..._EVIDENCE_DIR`；runner从output path派生一个同样事前不存在、repo外、无symlink parent的carrier/scratch root，
  并向child只注入冻结后的`..._CARRIER_ROOT`、shared-target内的`..._RUNTIME_BIN`及审计过的字符串环境；
- workload API：`phase6ScenarioSpecs(root)`、单一transitive `phase6WorkloadSpecs(root)`、
  `phase6CandidateSpecs(root)`、`phase6WorkloadProvenance(root)`、`phase6BoundedWorkLedger(root)`和coverage
  assertions；
- `phase6WorkloadSpecs(root)`把`phase5WorkloadSpecs(root)`返回的spec复制为
  `phase-5-regression-<old-id>`，记录`parentPhase = 5`与`parentId = <old-id>`，追加`phase-5-regression` lane，
  并原样保留command/cwd、`testFormat`、原lanes和已有`expectedTests`。唯一允许的args
  归一化是：当且仅当`command === cargo && args[0] === test`时，在`test`后幂等插入一次`--no-fail-fast`；target、
  filter和harness args完全不变。build/fmt/clippy不得插入该flag；它不运行旧Gate、不读取旧receipt；
- G6维护candidate-owned、逐entry显式列举的inherited provenance catalog；不得从嵌套id前缀推导source。
  `phase6WorkloadProvenance(root)`为每个final spec返回`sourcePhase/sourceId`、immediate `parentPhase/parentId`及完整
  有序`originChain[{phase,id}]`。Coverage assertion要求workload与provenance一一对应、chain末端是final Phase 6
  id、phase严格递增且source phases 1–6均有真实entry；unknown/missing/duplicate catalog row一律FAIL；
- `phase6BoundedWorkLedger(root)`按稳定obligation映射到同一transitive spec ids：Phase 1 dispatch/fuel、Phase 2/3
  cleanup/unwind、Phase 4 wake/claim、Phase 5 stream pump/buffer、Phase 6 materialization/root-walk。Gate机械断言每项
  至少一个实际spec且id存在；缺项重开原Phase owner，不能由Phase 7补enforcement；
- 本Phase所有`testFormat != null` workload（Node TAP、exact/filtered/unfiltered Rust）必须有positive integer
  `expectedTests`并精确匹配parsed summary；summary还必须证明failed/ignored/measured/skip/todo/cancel为0。
  Inherited specs若字段缺失或为`null`，Phase 6不得伪造：planning snapshot中的Phase 1–4 entries普遍缺字段，
  Phase 5 exact entries依赖`rust-exact`隐含的1，且`k5-scheduler-resource-authority`、
  `k5-capacity-one-stream-lifecycle`显式为`null`。A0必须从accepted Phase 5 API重新计算逐entry精确清单，handoff交给
  P7G显式adapter/catalog；
- 每个`cargo test` invocation在Cargo参数侧带且只带一次`--no-fail-fast`；build/fmt/clippy不带。Gate runner
  捕获nonzero后继续剩余spec。不得把
  shell `set -e`、Promise rejection或第一个失败变成outer fail-fast；
- runner在`/tmp/skiff-bcvm-p6-r1-cargo.lockdir`独占整个Cargo workload段，并设置共享target；禁止`cargo clean`；
- exact candidate在preflight、postflight、closure和fresh四点校验commit/tree/status；输出目录必须是caller
  选择的canonical absolute且事前不存在，不能位于candidate worktree；
- receipt/manifest绑定spec id、sourcePhase/sourceId、parentPhase/parentId、originChain、command/args/cwd、
  `expectedTests`的missing/null/integer三态、selected environment、exit、parsed summary、stdout/stderr hash、
  candidate commit/tree、schema、ISA、compiler artifact identity、image constructor identity和evidence epoch；
- checker拒绝zero/skip/todo/ignored/cancelled、缺receipt、重复id、stale candidate、wrong selector、missing lane、
  expectedTests漂移、command/env mutation、path escape、symlink/tamper/hash mismatch和worktree变脏。

首日expected-red必须在production join前完整运行一次：所有spec都执行并有receipt；至少一个本Phase真实断言red，
但不能因zero-hit、skip、fixture不存在、compile error未断言或runner提前退出而red。每个producer join后复用同一矩阵
看多个red同时收敛，禁止用单个fail-fast E2E逐问题发现。

## 7. Acceptance 与 handoff

### 7.1 Intermediate checkpoints

J0、J1和每个capability Gate只是rolling checkpoint，不是Phase acceptance，不生成可供Phase 7复用的accepted
receipt。任何production/test/fixture/Gate/observation/schema变化都开启新evidence epoch；旧PASS不能移植。

### 7.2 Freeze checklist

- [ ] activation记录的是Phase 5 `accepted` exact clean commit/tree，版本全部动态读取；
- [ ] compiler facts、atomic image、flat trampoline、request mux和memory ledger各只有一个authority/owner；
- [ ] verifier crate/stage/API/facts/seal/selectors/compat seam及linker semantic reconstruction为零；
- [ ] service、local+remote interface、same-Runtime callback、recoverable、DB、function task、Actor、Actor task均为
  `accepted`，或已经过Contract amendment明确降为`disabled`；
- [ ] disabled surface的S1/S4/S5/S6 negative Gate证明fail closed；
- [ ] success/throw/Pending/cancel/deadline/disconnect/partial allocation/late/duplicate与跨build/fence矩阵全绿；
- [ ] MemoryLedger peak/release、owner/root/resource/heap/image pin计数在每条terminal后归零；
- [ ] `phase6WorkloadSpecs(root)`含本Phase矩阵和完整Phase 1–5 workload regression，nonzero/non-skip且无stale/tamper；
- [ ] merged preflight后freeze exact clean commit/tree；全新只读semantic review cohort按facts/image、runtime
  ownership/memory、proof/false-green三个互斥主题并行，汇总一个verdict；另一名未写候选的Acceptance owner同时在
  detached clean worktree运行完整Gate并核对raw evidence。

最终result只有在semantic cohort与独立Acceptance都PASS后写`status = accepted`。`candidate`、`candidate-pass`、`gate-pass`均不能
传给Phase 7。

### 7.3 Phase 7 handoff record

Phase 6 result必须显式导出：

1. result内记录exact frozen implementation candidate commit/tree与Acceptance evidence；result/status-only commit和
   最终main merge完成后，handoff另报其exact commit/tree与clean status。Phase 7 A0从Git动态记录实际accepted
   closeout baseline，并证明candidate→baseline只含result/status allowlist、没有production/test/fixture/Gate/schema变化；
2. canonical selector、`phase6WorkloadSpecs(root)` API、Gate spec/manifest/evidence schema identity；
   `phase6WorkloadProvenance(root)`的逐spec P1–P6 origin chain，以及`phase6BoundedWorkLedger(root)`的稳定
   obligation→spec-id映射；
3. production compiler生成的真实artifact identity，以及从候选动态读取的schema/ISA；
4. `DeploymentExecutionImage`唯一atomic constructor及image identity；
5. capability ledger逐项列service、task-function、task-Actor、interface-local、interface-remote、callback-same-runtime、
   callback-cross-runtime、Actor、DB、recoverable、request-GC、Actor-compaction状态，不把interface/callback/DB/
   recoverable变体合并成一个模糊状态；
6. 每个accepted lane的pending/root/resource/child heap/boundary staging/memory peak-release/Actor arena观察；
7. disabled/deferred surface的唯一拒绝点和negative receipt；
8. residual owner mapping。任何语义缺口回到本Phase对应F6/K6/X6或capability owner，Phase 7不得实现；
9. inherited Phase 1–5 workload specs缺positive `expectedTests`字段的精确清单，供P7G显式adapter/catalog；
10. bounded-work ledger中P1 dispatch/fuel、P2/3 cleanup/unwind、P4 wake/claim、P5 stream pump/buffer、P6
    materialization/root-walk的canonical spec ids；任一缺项已在Phase 6 acceptance前reopen原owner。

Phase 7在自己的exact candidate上重新运行Phase 1–6 workload specs；它不嵌套Gate、不信任本Phase旧receipt、不固定
literal schema/ISA，也不决定本Phase未决定的GC、DB、recoverable、boundary或memory semantics。

## 8. 风险、非目标与停止条件

主要风险：central F6/K6/X6形成长尾；capability leaf误改shared state；legacy `RuntimeValue` boundary被误当VM
proof；TaskStore与DB被误宣称原子；Actor shared arena与request heap混淆；为追求并行建立第二child loop；测试只证明
手造对象；GC被用来掩盖root/memory缺口。MAP通过持久central owner、wave并行、first-day全矩阵和owner退回机制
控制这些风险。

本Phase非目标：cross-Runtime reverse callback、cross-service behavior envelope、exactly-once task、DB与TaskStore
分布式transaction、Actor live heap跨build迁移、general tracing/moving GC、`concurrent`/`serial`、性能优化或旧artifact
compatibility。

出现以下任一情况立即停止相应lane并先Amend Contract/MAP：需要linker/verifier推导source semantics；需要raw handle
跨heap；需要第二owner/root/budget/memory/child loop；需要让integrator或Gate owner顺手修production；需要Router
reverse-callback transport；需要扩大文件写集或改变依赖；需要把required语义推给Phase 7；或当前错误只能靠
message substring/手造image证明。
