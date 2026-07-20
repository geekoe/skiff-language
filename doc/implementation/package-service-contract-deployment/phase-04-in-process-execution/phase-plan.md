# Phase 04：In-Process Execution Plane 实现计划

状态：active；R01 PASS；R02在`ae7b601`第四次独立复验PASS，Wave 3 entry/checker已解锁

权威设计输入：`doc/architecture/package-service-contract-deployment.md`，重点 §2、§6、§7、§8、§9、§10、
§12、§14、§15。本文只冻结 Phase 04 的实现 DAG、写入 ownership、候选成熟度和验收证据，不定义
ServiceContract authoring、registry/release、deployment YAML 或 RemoteBoundary。

## 1. 阶段完成态

阶段验收时必须同时成立：

1. runtime 从 admitted `RuntimeAssembly` 构造 assembly-wide immutable execution image；package code按
   `PackageBuildId`只链接一次。canonical `PackageCallable`解析为 activation-independent direct target，
   canonical `ServiceCall`保持 activation-relative，不包装成 `ServiceUnit`、`PackageUnit`或 legacy
   `EvalRuntimeProgram`。
2. 每个 deployment / assembly generation拥有独立 `ActivationContext`。它显式拥有 binding/config/state/
   resource视图、request generation与callback capability table；共享 package build不得共享 mutable activation owner。
3. physical service binding只有 `InProcessBoundary`。内部调用按
   `(callerPackageBuildId, serviceRequirementSlot)`选择provider，切换provider context，按canonical contract
   value plan detached materialize参数、返回、typed error与stream item；缺本地provider直接失败，不经router。
4. `ActivationContext`随Rust future、owned continuation、stream producer/consumer、callback与cancel显式传播；
   production没有thread/task-local current service。返回receiver后恢复receiver context。
5. request-scope `any I`/native adapter只投影成opaque callback capability；capability包含runtime、activation、
   request generation、contract与opaque id，不携带method table/native object/address。request结束、stream关闭、
   cancel或owner退出后稳定返回`CapabilityExpired`/`CapabilityUnavailable`，不重建、不fallback，也不能进入
   DB/spawn/queue/recoverable lane。
6. package direct call继续复用同一heap/context，保留alias、identity与原地mutation，不经过service materializer。
7. ingress与internal service call进入同一个contract/binding dispatcher。host只从一个active assembly generation
   解析canonical ingress；request path不按build/operation/display name fallback，也不lazy-load artifact。
8. router拒绝runtime-originated service relay，不再替service call选择runtime、lazy-load provider或维护remote
   forward生命周期；外部gateway、actor/spawn和其它非service控制语义保持原有owner。
9. production旧`ServiceDependencySymbol -> OutboundServiceDispatch -> router`执行边不可达；没有legacy/dual path、
   remote placeholder或compatibility adapter。

## 2. 阶段输入与实现边界

Phase 04 的架构输入从typed `RuntimeAssembly`开始。Phase 02已经明确：当前语言尚不支持的lane使用tagged
unsupported状态，Phase 05才接authoring UX。因此：

- ordinary/error/stream/callback/native动态证据必须使用真实`ServiceContract`、`PackageArtifact`、
  `ServiceDeployment`、assembly resolver、typed loader/linker/admission和production dispatcher；禁止手写
  resolved provider target或绕过projection/admission。
- callback/stream source spelling与compiler UX不是本阶段完成条件；runtime任务不得修改compiler以发明语法。
- 若typed canonical artifacts仍无法经过deployment/assembly production validator到达runtime，属于本阶段
  blocker；不能用直接构造内部dispatcher对象代替全链证据。
- request wire可以从现有binary HTTP/WebSocket metadata严格投影`IngressSelector`；本阶段不冻结新YAML/CLI。
  `buildId`/`operationAbiId`即使仍存在于Phase 05待迁移wire，也不得参与canonical assembly target fallback。

## 3. Shared kernel checkpoint

Wave 1 合成一个高风险共享检查点，后续lane不得自行扩公共语义：

```text
AssemblyExecutionImage
  shared package code/type/executable image
  canonical package direct targets
  activation-relative ServiceCall instructions

ActivationContext
  assembly identity / generation / runtime replica / deployment owner
  immutable implementation + binding/config/state/resource views
  activation-owned request/callback lifecycle state

RequestActivationContext
  explicit receiver/provider owner + request generation + cancel/stream lifetime

InterfaceCarrier::CallbackCapability
  runtime replica / owner activation / request generation / interface-or-adapter contract / opaque id
  no method table, native object or address

InProcessBoundaryKernel
  resolve caller build + slot -> provider activation + contract operation
  plan-aware detached materialization
  typed ordinary/error/stream/callback lane handoff
```

`ServiceContract`仍是descriptor/schema/value-plan唯一owner；execution image、activation和dispatcher只保存exact
ref、operation ID或borrowed/Arc view。callback table按activation/request generation隔离，不能进入shared image或按
PackageBuildId缓存。物理binding不定义`RemoteBoundary` variant。

## 4. 三波 DAG

```text
Wave 1：shared kernel checkpoint
  T01 canonical assembly execution image ───────────────┐
  T02 ActivationContext/materialization/capability core ├─► T03 kernel eval handoff ─► R01
                                                        │
  R01 FAIL repair loop：F02 execution projection / F03 capability cleanup / F04 linker validation ─► R01 retry

Wave 2：R01 PASS 后三个lane并行                         │
  T04 ordinary/error + package-direct contrast ─────────┤
  T05 async/stream/cancel ───────────────────────────────┼─► R02
  T06 callback/native capability ────────────────────────┘
  R02 FAIL repair loop：F06 shared materialization + F07 callback mapping ─► F08 async/stream integration
                         └─► F09 stream terminal/drop cleanup ─► F10 pull/file cleanup ─► R02 retry

Wave 3：R02 PASS 后三个非重叠owner并行
  T07 host/request ingress + unified dispatcher ─────────┐
  T08 router runtime-service relay retirement ───────────┼─► T09R merged-production registration ─► R03
  T09 execution boundary checker/self-test ──────────────┘
       └─► T10 stable-candidate integration gate ─► A01 independent stage acceptance
```

T01/T02可并行，因为前者只拥有immutable code projection，后者只拥有runtime owner/materializer；T03只在二者
合流后建立最小eval/context接口和预声明lane模块。Wave 2各Agent只能实现自己的模块；`eval_context.rs`等中央
match/wiring由T03冻结并由T10做bit-identical集成，不能让三个lane争抢同一文件。Wave 3中runtime host、router、
checker写入域互不重叠。

## 5. 任务索引

| ID | 任务 | 依赖 | 风险 / 验收组 |
| --- | --- | --- | --- |
| D01 | [Independent phase-plan review](tasks/P4-D01-phase-plan-review.md) | 文档 checkpoint | 只读；执行前 gate |
| T01 | [Canonical assembly execution image](tasks/P4-T01-assembly-execution-image.md) | D01 PASS | 高；kernel checkpoint |
| T02 | [Activation/materialization/capability core](tasks/P4-T02-activation-boundary-kernel.md) | D01 PASS | 高；kernel checkpoint |
| T03 | [Kernel eval handoff](tasks/P4-T03-kernel-eval-handoff.md) | T01、T02 | 高；共享 API integration |
| F01 | [Package-test call-target exhaustiveness](tasks/P4-F01-package-test-call-target-exhaustiveness.md) | T03 host fixture compile blocker | 低；T01 API fallout repair |
| F02 | [Assembly execution projection repair](tasks/P4-F02-assembly-execution-projection.md) | R01@`ef14a08` blocker 1 | 高；T03 owner repair |
| F03 | [Capability cleanup/rollback repair](tasks/P4-F03-capability-cleanup-rollback.md) | R01@`ef14a08` blocker 2 | 高；T02 owner repair |
| F04 | [Assembly linker call validation repair](tasks/P4-F04-assembly-linker-call-validation.md) | R01@`ef14a08` blocker 3 | 高；T01 owner repair |
| F05 | [Eval callback projection ABI integration](tasks/P4-F05-eval-callback-projection-abi.md) | R01@`9eaea40` merge regression | 中；T03 integration repair |
| R01 | [Kernel checkpoint acceptance](tasks/P4-R01-kernel-acceptance.md) | T03、F02–F05 exact merged commit | 高风险只读 gate |
| T04 | [Ordinary/error execution](tasks/P4-T04-ordinary-error-execution.md) | R01 PASS | 高；lane batch |
| T05 | [Async/stream/cancel execution](tasks/P4-T05-async-stream-cancel.md) | R01 PASS | 高；lane batch |
| T06 | [Callback/native capability execution](tasks/P4-T06-callback-native-capability.md) | R01 PASS | 高；lane batch |
| F06 | [Shared boundary materialization](tasks/P4-F06-shared-boundary-materialization.md) | R02@`ee1609c` blocker 2 | 高；T04 owner repair |
| F07 | [Canonical callback interface projection](tasks/P4-F07-canonical-callback-interface-projection.md) | R02@`ee1609c` blockers 1、4 | 高；T06 owner repair |
| F08 | [Async error/stream capability integration](tasks/P4-F08-async-stream-capability-integration.md) | F06、F07 merged | 高；T05 cross-lane integration |
| F09 | [Stream terminal/drop cleanup](tasks/P4-F09-stream-terminal-drop-cleanup.md) | R02@`9809dee` lifecycle blocker | 高；T05 terminal ownership repair |
| F10 | [Pull/file consumer cleanup](tasks/P4-F10-pull-file-consumer-cleanup.md) | R02@`484cab0` terminal blockers | 高；pull/native consumer repair |
| R02 | [Execution lanes acceptance](tasks/P4-R02-lanes-acceptance.md) | T04–T06、F06–F10 exact merged commit | 高风险只读 gate |
| T07 | [Unified ingress/internal dispatcher](tasks/P4-T07-unified-ingress-dispatch.md) | R02 PASS | 高；entry batch |
| T08 | [Router service-relay retirement](tasks/P4-T08-router-service-relay-retirement.md) | R02 PASS | 高；entry batch |
| T09 | [Execution boundary checker](tasks/P4-T09-execution-boundary-checker.md) | R02 PASS | 中高；并行 checker/self-test implementation |
| T09R | [Merged production registration](tasks/P4-T09R-execution-boundary-registration.md) | T07–T09 exact merged commit | 中高；production checker checkpoint |
| R03 | [Entry/remote-retirement acceptance](tasks/P4-R03-entry-remote-acceptance.md) | T09R exact commit | 高风险只读 gate |
| T10 | [Phase integration gate](tasks/P4-T10-phase-integration.md) | R01–R03 PASS | 唯一昂贵 gate owner |
| A01 | [Independent stage acceptance](tasks/P4-A01-stage-acceptance.md) | frozen T10 candidate | 独立只读验收 |

## 6. 写入 ownership

- T01独占`runtime/linked-program/**`、`runtime/linker/**`、`runtime/linked-type-plan/**`中的assembly execution
  projection。不得修改activation/boundary/eval/request/host/router。
- T02独占`runtime/model`的`InterfaceCarrier::CallbackCapability`与exhaustive model clone/graph seam、
  `runtime/boundary`新service-linkable materializer、`runtime/activation`新context/binding/capability modules；同时
  拥有`binary`/`recoverable`、`runtime/service-db`及供spawn/queue共用的persistent boundary最小拒绝delegate与
  测试，保证callback carrier在所有persistent lane fail closed。新逻辑必须拆新模块，旧大文件只接delegate
  match，不复制实现。
- T03独占`runtime/eval`的assembly execution seam、显式context carrier与lane hook/module shell，以及必要crate
  exports；它还独占host admission tests下共享typed execution fixture/harness与三个预声明lane test文件。T03在
  `eval_context`冻结callback capability到T06 hook的exhaustive delegate（checkpoint typed fail closed）。T04–T06
  不得再改T03冻结的中央dispatch或fixture root。
- T04独占ordinary/error lane模块与canonical package/service call executor；不得修改stream/callback模块。
- T05独占async/stream/cancel lane模块、现有stream runtime/cancellation owner的必要最小改动；不得修改ordinary/
  callback或中央wiring。
- T06独占callback/native lane、callback table consumer、native explicit adapter；不得修改ordinary/stream或
  compiler authoring。
- T07独占`runtime/request/**`、`runtime/host/**`的canonical request entry、active-generation context set和旧
  outbound service injection删除面，以及必要`runtime/transport`严格selector projection；不修改router。
- T08独占`router/**` runtime-originated service relay拒绝/删除面；保留gateway与非service control flow。
- T09独占execution boundary checker engine与hermetic self-test；其独立branch允许报告T07/T08尚未合流导致的已知
  production violations，不得伪报零。T09R在三分支合流后独占真实subject registration、production checker、
  verify接线与零违规证据，不修改Rust/TypeScript production。
- T10只做集成、机械compile/test seam、证据汇总与结果草案；语义缺口退回原owner。

已经很长的`runtime/boundary/src/binary.rs`、`recoverable.rs`、`runtime/eval/src/eval_context.rs`、
`program_execution.rs`和router dispatcher/registry不得继续接收新execution owner；新职责进入按lane拆分的模块。

## 7. 最早风险探针

### Kernel / ordinary

- 两个activation复用同一package code `Arc`，但context、binding/config/state/callback table地址不同；不同package
  的slot 0不会冲突。
- service参数provider侧handle与caller不同；provider mutation不影响caller；return/error再次detached。
- package direct同一fixture保留相同handle、alias和原地mutation，证明未被统一强制linkable。
- missing binding/provider/operation/protocol/schema/plan在invoker前失败，panic router spy保持零调用。

### Async / stream / cancel

- provider在suspend前后读取相同provider owner；返回/throw后receiver恢复caller owner。
- stream producer每次emit处于provider context，consumer处于receiver context；每项detached；early break/close/cancel
  终止producer并清理registry/lifetime。
- pre-cancel、pending unary、pending stream next、owner exit均exact-once terminal，无task/table泄漏。

### Callback / native

- callback进入capability owner，返回恢复provider/receiver；wrong runtime/activation/generation/interface、request end、
  stream close、cancel、owner exit稳定映射设计规定错误。
- ordinary detached plan遇到local interface/native handle必须通过显式callback adapter，否则fail closed；opaque
  capability不携带method table/native address。
- capability进入DB/spawn/queue/recoverable编码必须失败，不调用rebuild/fallback hook。
- T03共享fixture必须从`ServiceContract`/`PackageArtifact`经过deployment projection、assembly resolver、typed
  load/link/admit取得execution target；T04–T06分别在预声明lane文件增加正负例，不能手写resolved provider。

### Entry / remote retirement

- ingress与internal call的production代码引用同一个dispatcher symbol；active generation pin贯穿整个request。
- request只按canonical selector查active assembly；buildId/operationAbiId/display target变异不改变target或直接拒绝，
  不能fallback旧route registry。
- runtime发起canonical service call时router无`request.start`；router收到`caller.kind=service` relay帧直接拒绝，
  gateway与actor/spawn回归仍PASS。
- checker枚举所有执行user code的`tokio::spawn`并验证owned context；current-service TLS、第二dispatcher、callback
  shared/recoverable owner、旧outbound service call reachability都能通过mutation self-test检出。

## 8. 验证计划

开发Agent只运行targeted rustfmt、直接crate/filter测试和结构探针。T10是最终稳定候选的唯一昂贵gate owner：

```bash
node scripts/verify.mjs --only runtime
node scripts/verify.mjs --only router
node scripts/verify.mjs --only type-check
node scripts/verify.mjs --only checks
node scripts/check-runtime-crate-dag.mjs
node scripts/check-runtime-artifact-boundaries.mjs
node scripts/check-runtime-execution-boundaries.mjs
git diff --check
```

若`checks`已展开三个显式checker，ledger只记录一次实际执行，不重复调用。T10还必须在同一候选运行真实
provider/consumer typed full-chain、package-direct same-heap对照和in-process runtime smoke；smoke不得依赖Phase 05
authoring/registry adapter。按workspace规则，影响chat链路的runtime候选合入main前还需构建stable runtime并运行
`internals/agine` chat smoke；若Phase 05尚未迁移的consumer精确fail closed，记录为跨阶段预期失败，但不能把它
冒充Phase 04通过证据。

Phase 04不运行telemetry或跨仓完整gate；没有修改compiler时保留Phase 03/Phase 02的foundation/compiler证据，
但T10必须用exact diff证明未触及对应owner。router/runtime/checker/public API/Cargo变化会使相关证据失效。

## 9. 稳定候选与验收

- R01在T01–T03 exact integration commit上验收execution image、ActivationContext、materializer、capability table和
  frozen lane seams；PASS后才扇出T04–T06。
- R02对T04 ordinary/error、T05 async/stream/cancel、T06 callback/native分别给出verdict，并检查三者只通过T03
  kernel交接；PASS后才开始entry cutover。
- T09先在独立branch完成checker/self-test；T07/T08/T09合流后由T09R注册真实owner并要求production零违规。
  R03在T09R exact commit上验收single dispatcher、active-generation ingress、router service relay retirement与
  checker mutation coverage；PASS后进入收敛模式。
- T10先完成阶段标准→真实入口/动态或结构证据/关键负例/owner/精确commit覆盖检查，再冻结stable candidate。
- A01只对冻结candidate给出PASS/FAIL。任何production、Cargo、checker、fixture或gate环境变化都结束当前
  stability epoch；blocker批量归类后退回对应owner。

## 10. 非目标与停止条件

非目标：ServiceContract/deployment YAML/CLI authoring、registry/storage/release pointer、旧service/package-test
tooling迁移、RemoteBoundary、跨assembly调用、service级进程隔离或独立扩缩容。

以下任一情况暂停受影响DAG并升级：

- 需要让service call按provider package/build/display target或router runtime registry选择，而不是caller activation binding。
- 需要把callback method table/native object/address放进capability wire，或让capability进入recoverable lane。
- 需要thread/task-local current service、全局slot patch、共享mutable activation owner或request-time artifact load。
- 需要legacy/remote fallback维持Phase 05 consumer可运行，或给canonical call建立dual path。
- 需要改变四对象、package direct same-heap、service boundary detached语义或callback lifetime/error公共契约。

纯Rust模块布局、内部trait/error类型、context set/cache策略、严格wire到`IngressSelector`的确定性投影由任务owner
在上述设计内决定，不升级为产品决策。
