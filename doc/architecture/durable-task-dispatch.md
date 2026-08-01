# Durable Task Dispatch Architecture

本文定义 Skiff detached call 的目标态内部架构契约：持久任务提交、定时可见、claim / lease、
at-least-once 恢复、取消、Runtime 派发和执行身份保留。本文只描述目标态；现状到目标态的迁移属于实现计划。

用户可见关键字固定为 `dispatch`；具体 grammar、公开类型名和 cancel result 拼写仍应落到 `../reference/`。本文不定义
scheduler policy 配置格式、物理 TaskStore / ready queue 产品或 service DB 与任务提交的业务原子性；内部统一把该语义操作称为
**task dispatch**，把持久记录称为 **task**。旧 `spawn` surface 被 `dispatch` 完整取代，不保留兼容语法、旧 artifact
分支或第二条易失执行路径。

## Position

task dispatch 表达“提交一个与当前 request 生命周期分离的 callable invocation”。立即执行和延迟执行是同一个
机制的不同 eligible time：立即任务的 `due_at` 是提交时刻，延迟任务的 `due_at` 是未来时刻。平台不为立即任务
保留一条易失 direct-spawn 语义线，也不让 Runtime request 通过 sleep 占用并发槽来实现延迟。

所有成功提交的 task 必须先成为持久调度事实，随后才可以创建 Runtime request。平台可以在 durable commit 后立即
唤醒本地 Runtime，也可以把 task 派发给其他合格 Runtime；这种 fast path 不能绕过 task store、claim、lease 或
terminal settlement。

核心契约：

- task placement 位置透明。提交方、执行方是否位于同一 Runtime replica 不是语言事实。
- task 使用持久 payload；提交前必须完成 recoverable encode，失败时不得创建半截 task。
- task store 成功确认后，task 在取消或 terminal 前保持可调度；Runtime / Router 重启不能把已接受 task 静默变成不存在。
- task 提供 at-least-once execution attempt。结果不确定的 attempt 可以产生后续 attempt，因此不承诺 exactly-once effect。
- 同一 task 同时最多有一个有效 lease；旧 lease 不能提交 task 状态，但 fencing 不能撤销外部副作用。
- task identity 自动吸收 ready notification / due scanner 的普通重复触发，不提供业务 dedupe key，也不合并两次独立提交。
- task dispatch 是独立平台 capability，不自动参加 service DB transaction。业务数据写入与 task 提交之间的一致性由业务
  reconciliation、显式 outbox 或未来单独设计的 bridge 处理，不属于本机制的隐式保证。

## Layer Ownership

长期阶段流向：

```text
source `dispatch` operation
  -> compiler target / payload / timing validation
  -> linked task-submit plan
  -> recoverable payload encode
  -> durable TaskStore commit
  -> scheduler visibility / claim
  -> exact execution-image selection
  -> Runtime request admission
  -> attempt settlement
  -> durable task terminal state
```

各层职责固定如下：

- compiler 证明 target 合法、return 为 void、参数满足 recoverable boundary，并生成精确 callable 与 payload plan；compiler
  不选择 Runtime、不理解物理 scheduler schema。
- Runtime submission side 求值 receiver、参数和 timing，编码 payload，并通过 task capability 提交；它不能直接创建一个
  未经持久化的派生 request。
- TaskStore 是 task identity、状态、`due_at`、attempt、lease、取消结果和 terminal outcome 的权威 owner。
- scheduler 只从 TaskStore 的可见事实选择工作，负责 timing、fairness、capacity、claim 和 Runtime candidate selection；它
  不解释业务 payload。
- Router / runtime transport 把一次有效 attempt 映射为一次 request frame，并把 terminal outcome 归还给 task settlement；
  transport `requestId` 只标识该 attempt 的 frame，不是 task identity。
- Runtime execution side 按精确 linked target decode payload 并执行 callable；它不能从 target 名字、latest build 或 payload
  自带的历史 descriptor 猜测执行计划。

TaskStore 可以由数据库、日志、带延迟能力的 broker、自研 store 或其组合实现。物理产品不是 artifact、service config 或源码
语义；任何 adapter 都必须提供本文要求的 durable create、conditional transition、due-time visibility、lease fencing 和
terminal retention。普通 FIFO queue 不具备完整 contract；它最多承载 task 到期后的 ready notification。

### Control-plane owner and authority

Task control plane 是 TaskStore、scheduler 与 Runtime task capability 的逻辑 owner。它可以与 Router 共进程，也可以独立部署；
这种物理位置不改变以下信任边界：

- Runtime submission side 只能使用当前受认证 ActivationContext 注入的 task capability，不能提交任意 owner、environment、
  execution image 或 target。
- task control plane 必须把提交方 authority 精确投影成 task owner 与 trusted execution witness；TaskStore 不接受 Runtime 自报的
  service/build/display name 作为 authority。
- scheduler 只能从 TaskStore 读取已经认证并 durable commit 的 record，再用 record 中的 trusted witness 请求 Runtime admission；
  它不能从 payload 或 ambient active deployment 补 execution identity。
- 多个 scheduler replica 可以并发扫描和派发，但只能通过 TaskStore conditional claim 获得 lease；leader election不是正确性
  前提。
- Runtime 只接受 task control plane 签发、与当前 lease / AttemptId / execution witness 一致的 request start。原提交 Runtime
  消失不影响后续 scheduler 构造受认证 attempt。

## Durable Timing And Ready Delivery

task dispatch 的核心不是传统 queue，而是 **durable task scheduler**。TaskStore 的权威事实是 task record、`due_at`、state、
lease 和 terminal outcome；scheduler 必须能按 `due_at` 找到已到期且仍可执行的 task。

`scheduled` 与 `ready` 是逻辑 visibility，不要求物理搬运消息。adapter 可以采用任一等价形态：

1. TaskStore 提供 `(state, due_at)` 索引，worker 直接 conditional-claim `due_at <= now` 的 task，不存在独立 ready queue。
2. due scanner 把到期 TaskId 发布到 ready queue，worker 收到 notification 后仍回 TaskStore claim。
3. 底层 broker 原生支持 durable delayed visibility，到期投递仍以 TaskStore state / lease 为权威。

第二种形态中的 ready queue 只是唤醒和负载分发加速器：notification 可以重复、迟到或丢失，scanner 必须能重建；notification
不拥有 payload、cancel state、lease 或 terminal truth。删除 ready queue 后，系统仍能从 TaskStore 恢复全部未完成工作。

立即 task 与延迟 task 使用同一模型。立即 task 写入 `due_at <= now`，提交成功后可以主动 wake scheduler；延迟 task 写入未来
`due_at`，由 durable due-time index 使其到期可见。二者都不是 Runtime 内存 timer，也不通过提前创建 request 再 sleep 实现。

## Canonical Task Identity And Record

每次 source-level 提交生成一个新的 opaque `TaskId`。TaskId 在第一次物理写入前生成；平台因提交响应丢失而重试同一次
内部写入时必须复用该 TaskId。两个独立执行到的提交操作即使 target 和参数完全相同，也生成两个 task；平台不做 payload
hash dedupe、业务 key dedupe 或 target-level coalescing。

目标态 task record 至少包含以下事实；结构只表示字段所有权，不冻结物理 schema：

```rust
struct TaskRecord {
    task_id: TaskId,
    owner: ServiceOwner,
    execution: TaskExecutionImageRef,
    target: DetachedCallTarget,
    payload: RecoverablePayload,
    due_at: DurableUtcTimestamp,
    state: TaskState,
    attempt_generation: u64,
    active_lease: Option<TaskLease>,
    terminal: Option<TaskTerminal>,
    trace: TaskTraceContext,
    created_at: DurableUtcTimestamp,
}

enum DetachedCallTarget {
    Function(ExecutableIdentity),
    ActorMethod {
        actor: ActorIdentity,
        activation: ActorActivationSnapshot,
        implementation: ActorImplementationIdentity,
        method: ActorMethodIdentity,
    },
}
```

`TaskExecutionImageRef` 的逻辑内容固定为：

```text
targetEnvironment
exact PackageVersion label
RuntimeAssemblyRef
RuntimeConfigSnapshotRef
ServiceDeploymentRef
```

其中 `RuntimeAssemblyRef` 闭合 exact `PackageBuildId` 和 executable graph；`ServiceDeploymentRef` 闭合 deployment revision。
PackageVersion label 必须作为 task 的显式、可观测版本事实保留，不能在执行时从 active deployment 反推。

`DurableUtcTimestamp` 是可持久化、可跨主机比较的 canonical UTC epoch timestamp，不是 Rust / Tokio 的进程内 monotonic
`Instant`。`due_at` visibility、lease expiry 与过期竞争都使用 TaskStore 的权威时钟。Runtime / scheduler 的 monotonic clock
只能用于本地提前 wake / renew，不能决定 durable not-before 或判定自己仍持有 lease。

absolute `at` timing 规范化为一个 UTC timestamp。relative `after` timing 以 source operation 求值 timing 时取得的平台 UTC
时间为基准，加上非负 duration；durable commit latency 不向后平移 `due_at`。若 commit 成功时 `due_at` 已经过期，task 立即
eligible。平台拒绝无法表示、overflow 或负 duration；zero duration 等价于 immediately eligible。

TaskId 与 attempt transport identity 分离：

- `TaskId` 在 task 整个生命周期中不变，也是取消、状态查询、trace correlation 和重复投递识别的 key。
- `AttemptId` 每次有效 claim 新建。
- `requestId` 每次 Runtime transport execution frame 新建。
- 重试必须保留 TaskId，原子推进 attempt generation，并获得新的 lease id / AttemptId / requestId。

取消和状态查询需要一个可恢复的 opaque task reference。其 public 拼写由 reference 文档决定；内部能力只保存 TaskId 与
owner scope，不保存 Runtime address、queue partition、lease id 或 mutable task snapshot。task reference 可以跨 request
恢复，但不能授予跨 owner service 操作其他 service task 的权限。

## Execution Image And Target Pinning

task 在提交时冻结完整 execution image，而不是只保存可读 target 名字并在到期时选择 latest：

```text
service owner
+ service version
+ exact build / assembly identity
+ deployment revision
+ exact Runtime configuration snapshot
+ exact function or actor-method identity
+ payload expected-type plan identity
```

`TaskExecutionImageRef` 是这些 immutable artifact / configuration facts 的受认证引用。它不包含提交 request 所在的物理 Runtime
replica，也不引用该 replica 当时的 activation generation：正常发布和 drain 必须能结束旧 Runtime，而不能因为一个很久以后的 task
把旧进程留住。到期 attempt 由 task control plane 针对 frozen image 建立新的 task activation，或选择已经 admission 同一 image 的
Runtime；这个 activation 拥有自己的 generation，并继续服从普通 Runtime admission 与 drain 规则。

因此“执行旧代码”固定的是可重新激活的旧 execution image，不是某个旧进程。非 terminal task 是其 image、配置 snapshot 和必要
artifact 的 retention root。发布、drain 和 artifact GC 不得让已接受 task 静默改用不同实现；若 operator 显式破坏 retention，
task 以可观察的平台失败收敛，不能 fallback 到 latest build。terminal transition 必须原子释放 execution image / artifact
retention root；后续 status / audit tombstone retention 不继续 pin executable
artifact。若审计需要长期保存代码，必须使用独立 artifact audit retention policy。

这一规则冻结代码和配置语义，但不把 artifact/build identity 复制进 recoverable payload。payload 仍按
[`recoverable-value.md`](recoverable-value.md) 的 owner-internal envelope 规则编码；execution image 是 task 调度元数据，
decode 使用该 image 已 admission 的 linked expected plan。

scheduled horizon 和 outstanding-task quota 同时限制旧 image 的最长 retention 与数量。配置 snapshot 若包含 secret capability，必须以
平台受保护引用保存并在 attempt 激活时重新授权，不能把明文 secret 复制进 task record 或 payload。

### Actor-method target

Actor logical identity 按 [`actor-model.md`](actor-model.md) 保持跨 service version 稳定；task 中的 service version 和
`ActorImplementationIdentity` 只决定这一次延迟调用要求的代码，不进入 ActorIdentity。actor-method task 不是保存一个易失对象指针，
而是保存以下可恢复事实：

- ActorIdentity；
- 提交时精确的 ActorImplementationIdentity 与 method identity；
- 从 actor registry entry 冻结的 key / create 输入和对应 expected-type plan，即 `ActorActivationSnapshot`。

`ActorActivationSnapshot` 只包含重新执行 `create` 所需的 recoverable 输入，不保存 Actor 内存字段。提交 actor-method task 前，平台必须
从当前受认证 actor handle / registry entry 取得并完整编码这份 snapshot；缺少 entry 或存在不可恢复的 create 输入时，提交失败，不能
创建一个以后必然无法激活的 task。

到期时 task control plane 执行的是 Actor 路由层内部的 **get-or-activate**，语义上复用 `std.actor.get` 的激活路径，但不是重新求值
一段用户源码：

1. live incarnation 与 task 的 implementation identity 相同：按普通 Actor admission 排队执行 method。
2. 没有 live incarnation，但 registry entry 仍存在：按 entry 保存的创建输入激活 task 固定的旧 implementation，再执行 method。
3. Router 重启等原因使 registry entry 丢失：用 task 持久保存的 `ActorActivationSnapshot` 恢复最小 entry，执行 `create` 后再调用
   method。
4. task implementation 是 Actor 升级控制器认可的 forward target：和普通调用一样触发或等待升级；临时
   `ActorUpgradingError` 属于可恢复平台错误，按退避形成后续 attempt。
5. live incarnation 或 registry fencing 已由不同的新 implementation 接管，而 task implementation 已被标记为旧实现：拒绝旧 task；
   不得切回旧实现，也不得把旧 payload 交给新代码。该 attempt 以 `ActorVersionRejectedError` 明确结束，task terminal 为
   `platform-failed`。

registry entry 已存在时永远沿用 entry 的创建输入，忽略 task snapshot，和 `std.actor.get` 的 put-if-absent 规则一致；snapshot 只用于
entry 缺失时重建。若 Router 重启前后产生的多个 task 为同一 ActorIdentity 携带了不同 snapshot，第一次成功恢复 entry 的操作获胜，
后续 task 不覆盖它。恢复 entry、普通 `get` 与 Actor owner claim 必须在同一个 identity fencing 下竞争，不能并发创建两个 live
incarnation。

旧 task 与升级请求竞争时仍以 Actor owner 的原子 admission / upgrade fencing 为准：旧 task 在 upgrade 关闭 admission 前已进入旧
incarnation，可以按普通旧方法完成；升级先关闭 admission，则旧 task 被拒绝。task lease 不提供第二套顺序，也不能使旧 task 触发
反向升级。

第三种情况不会恢复逐出或崩溃前的 Actor 内存；`create` 仍应按 Actor 模型从 service DB 等业务事实重建。actor-method task 保证的是
可靠投递 attempt，不是 Actor 内存快照或 exactly-once method effect。若 Actor 已不存在但没有不同 implementation 的 live owner，
允许从 task 固定的旧 image 冷激活旧实现；之后普通新版本调用仍可按 Actor 升级协议接管它。

## Submission And Visibility

提交顺序固定为：

1. 求值 timing、receiver 和全部参数；一次 source operation 中每项只求值一次。
2. 解析并冻结 exact execution image 与 target；actor-method target 同时冻结 ActorActivationSnapshot。
3. 按 target expected plan 完整编码 recoverable payload。
4. 生成 TaskId，原子 durable-create task record 并登记 execution image / artifact retention root。
5. TaskStore 确认 durable create 后，submission 才成功返回 task reference。
6. `due_at <= now` 的 task 进入 ready visibility；未来 task 保持 scheduled，直到 `due_at` 到达。

步骤 1–3 失败不得产生 task。步骤 4 的响应不确定时，平台只能用同一个 TaskId 重试 create；TaskStore 的 create 必须是
TaskId-idempotent，并拒绝同 TaskId 不同 canonical record 的冲突。task record 可见与 retention root 生效是同一个逻辑 commit：
不得出现 task 已接受但旧 image 可被 GC，或 image 已长期 pin 住但 task 并不存在的窗口。

submission outcome 必须区分 definite rejection 与 ambiguous acceptance。成功返回保证 TaskStore 中存在 exact TaskId；definite
rejection 保证没有创建 task；连接丢失或响应无法判定时，task 可能已 durable commit 并继续执行。内部仍持有同一 submission
context 时只能复用原 TaskId 查询 / 重试，不能生成第二个 TaskId。若整个 caller request 被外部重新执行，它会形成新的独立
task；本架构不提供业务 dedupe 来合并两次 source operation。

`due_at` 是 not-before boundary，不是准点 SLA。scheduler 按 TaskStore authority time 不得早于 `due_at` claim；到期后的实际
开始时间受容量、fairness、Runtime availability、actor ownership 和平台健康影响。wall-clock rollback / skew 的误差处理由
store adapter 统一界定：误差可以造成迟执行，不能让各 Runtime 按本地 wall clock 主动突破 not-before boundary。

## State Machine

canonical task state 为：

```text
scheduled ──due──────────────> ready
scheduled ──cancel───────────> canceled

ready ─────claim─────────────> leased
ready ─────cancel────────────> canceled

leased ────return────────────> succeeded
leased ────throw/reject──────> failed
leased ────permanent platform error──> platform-failed
leased ────lease loss────────> ready          (new attempt)
leased ────cancel────────────> leased          (AlreadyStarted)
```

`succeeded`、`failed`、`platform-failed` 和 `canceled` 是 terminal。`failed` 表示 target 明确 throw / reject；
`platform-failed` 表示平台已经证明当前 task 永远不能形成合法 execution attempt。物理实现可以把 `scheduled` / `ready`
表达为一个状态加 visibility index；对外 transition 与竞争结果必须等价。

每个 transition 都是 TaskStore 对当前 state / lease id 的 conditional write。scheduler、Runtime 和 duplicate delivery
不能无条件覆盖 terminal state，也不能让旧 lease 完成新 attempt。

## Claim, Lease And Fencing

scheduler 只 claim `due_at <= now`、state 为 ready、execution image 可重新激活且 policy 有容量的 task。claim 原子写入：

- state = leased；
- monotonically increasing attempt generation；
- fresh AttemptId / lease id；
- lease owner、lease expiry 和 selected execution image witness；
- task / service / key capacity accounting（若 policy 定义这些维度）。

Runtime 必须在 lease 到期前 renew。completion、failure 和 heartbeat 必须携带当前 lease id；
TaskStore 拒绝 stale lease 的所有 settlement。

lease expiry 与 settlement 在 TaskStore authority time 上竞争同一个 CAS：

- settlement 只在 `state = leased && lease_id = current && store_now < lease_expiry` 时成功；
- recovery 只在 `state = leased && store_now >= lease_expiry` 时成功，并原子清除 active lease 后转回 ready；
- 两者最多一个成功。旧 lease 即使 recovery 尚未被 scanner 观察，也不能在 authority time 过期后恢复或 settlement。

lease fencing 只保护平台 task state。旧 Runtime 在失去 lease 后仍可能完成已经发出的 HTTP、DB、文件、模型或第三方调用；
平台无法回滚这些 effect。新的 attempt 与旧 attempt 也可能在真实世界短暂重叠，即使 TaskStore 同时只有一个有效 lease。

## At-Least-Once Contract

成功提交且未取消、未 terminal 的 task 在 execution image 可用且基础设施最终恢复的前提下持续可调度。Runtime admission
只是一次 attempt 的开始，不是 logical task 的完成点。普通 ready notification 重复、due scanner 重复、scheduler 重试和
提交响应重试不得自行产生新的 logical task。

以下情形属于基础设施恢复，平台自动为同一 TaskId 创建后续 attempt：

- claim 后尚未得到 Runtime admission 就失去执行方；
- Runtime 断连、进程消失或 lease 过期，平台无法证明 attempt 已可靠 settlement；
- attempt settlement 响应不确定且 TaskStore 中没有相符 terminal transition。

基础设施恢复必须使用平台级 bounded backoff + jitter，不能因 unavailable Runtime、store outage 或 poison task 形成 hot retry。
这项 pacing 不等于用户可配置的 application-error retry policy。

平台错误必须先分类，不能把所有失败都当作 lease loss：

- 暂时性错误，例如当前没有容量、合格 Runtime 暂时不可用、transport 中断、TaskStore 短时不可用或 attempt outcome 不确定，
  保持同一 TaskId 并按退避产生后续 attempt。
- 已证明永久的错误，例如 task record / authority 损坏、target identity 永久不存在、payload expected plan 永久不兼容或 operator
  明确删除必需的 execution record，进入 terminal `platform-failed`，不得无限 hot retry。
- 第一版不设置基础设施 max attempts 或 task age expiry；只要错误仍可能恢复，task 就保持非 terminal。永久性判断必须来自
  canonical registry / identity / policy 事实，不能因为某次 Runtime 暂不可达而猜测。

target 明确返回时 task succeeded；target 明确 throw / reject 时 task failed。普通业务 failure 已经完成一次 execution attempt，
第一版不因为 target throw 自动重试。按错误类别、退避、次数或业务结果配置 automatic retry 是独立 policy，不从
at-least-once delivery 隐式推出。

若 target effect 已发生但 attempt 未写入 succeeded，lease recovery 会再次执行 target。这是本契约允许的重复，不存在通用
exactly-once effect 保证。Task ID 和 fencing 只能吸收调度层重复，不能把 arbitrary callable 与外部 effect 纳入同一个原子
commit。

## Duplicate Delivery Handling

平台必须让普通 ready delivery 重复对 target 不可见：

- duplicate notification 不创建第二个 logical task；同一 attempt generation 最多产生一个有效 lease。lease-loss recovery
  仍可为同一 TaskId 创建新的 attempt。
- task 已 leased 且 lease 有效时，其他 consumer 不能获得第二个有效 lease。
- task terminal 后到达的 notification 只用于清理 / ack，不能重新打开 task。
- due scanner 的重复 fire 通过 TaskId 与 state CAS 收敛，不能创建第二个 task。
- cancel 后到达的 notification 不能恢复 ready。

这不是业务 dedupe。两次独立 task submission 永远是两个合法 task，即使 payload 完全相同；架构不保存 `dedupeKey`，也不
提供“same target + same args”合并规则。

ready notification 不是权威记录；TaskStore 已无对应 TaskId 时必须 ack / drop，不能从 notification 重建 task。terminal metadata
按 status / audit policy 保留，不因 ready notification 生命周期被迫延长。TaskId 在 owner scope 内永不复用；terminal metadata
过期后，旧 task reference 的 status / cancel 返回稳定的 expired / unavailable 结果，绝不能解析成另一个 task。具体 public
result 拼写由 reference 定义。

## Cancellation

取消是对 TaskId 的持久状态操作，不是对某个 Runtime connection 的临时消息。第一版只提供 **before-start cancellation**：

- scheduled / ready task 的取消通过 CAS 直接进入 terminal canceled；成功返回后保证不会产生 attempt。
- cancel 与 claim 竞争同一个 state CAS。cancel 先成功则 task 永不 leased；claim 先成功则 cancel 返回 `AlreadyStarted`，不修改
  task state、不发送 stop hint。
- leased task、已经 terminal 的 task 和 retention 已过期的 task 分别返回可区分结果；具体 public union 拼写由 reference 定义。

因此延迟 task 在到期前通常可以可靠取消；立即 task 只有仍处于 ready、尚未 claim 时可以取消。第一版不公开运行中 cooperative
cancel，不改变普通 Runtime request 与 actor method 的现有停止契约。未来若需要 running-task cancellation，必须单独定义
cancel-requested、停止确认、outcome unknown 和副作用边界，不能扩张 `canceled` 的含义。

## Runtime Admission And Settlement

claim 本身不等于 target 已开始。scheduler 在获得 lease 后，必须针对 exact execution image 冷激活或向已经 admission 该 image 的
Runtime 发起普通
request admission。admission 失败且平台能证明 Runtime 未接受 request 时，可以释放或重新 claim；admission 结果不确定时按
lease-loss recovery 处理，允许产生重复 attempt。

一次 attempt 的 Runtime request：

- 使用 fresh request heap 和 request-local context；不继承提交方 call frame、timeout、cancel token、DB transaction、stream、
  live connection 或 mutable heap identity。
- 它是 target 对应类别的普通 request，复用相同的 Runtime admission、connection concurrency、heap / payload limit、instruction /
  continuous-execution budget、native effect guard、deadline / timeout 和内部 stop 机制；平台不为 task execution 建第二套资源模型。
- fresh attempt 不继承提交方已经消耗的剩余 deadline，而是按 target 在当前 execution context 中创建一个完整的新普通 request
  budget。task lease 只表达 attempt ownership / fencing，不充当 request execution deadline，也不能靠持续 renew 绕过普通 request
  timeout。
- 继承 task trace correlation，但创建新的 attempt / request span。
- function target 作为独立 service request 执行。
- actor-method target 通过 actor registry / owner fencing 的 get-or-activate 路径执行，并遵守 actor instance 的串行、升级与
  suspension 契约；task lease 不替代 actor owner lease，ActorActivationSnapshot 也不绕过唯一 live incarnation 约束。
- return value 必须为 void / null；平台只保存 task outcome，不保存 callable result。

Runtime 得到明确 outcome 后用当前 lease id settlement。若 terminal write 失败或响应不确定，Runtime 可以用同一 lease id 重试
相同 settlement；TaskStore 必须 idempotent 接受完全相同的 terminal write，并拒绝同 lease 的冲突 outcome。
普通 request deadline / timeout 是一次已经开始的明确 execution outcome，按 target 类别的普通错误投影收敛为 terminal
`failed`；它不是基础设施 lease loss，第一版不因此自动重跑 target。

## Backpressure And Fairness

durable submission 把 producer latency 与 execution capacity 解耦，但不能把容量问题变成无限 backlog。平台必须在两个边界
分别限流：

- submission admission：service quota、payload size、scheduled horizon、outstanding task count 和 store health；超限时提交失败，
  不能先返回成功再静默丢弃。
- execution admission：Runtime capacity、service / traffic-class fairness、target concurrency、actor ownership 和可选 key
  concurrency。

立即 task 只表示立即 eligible，不拥有绕过旧 ready task 的绝对优先级。scheduler policy 可以区分同步与异步 traffic class，
但 detached task 不能伪装成同步 request 绕过公平性。

本地 fast path 是 durable commit 后的 wake / claim 优化：提交方 Runtime 有容量时可以优先成为 candidate，但 TaskStore 的公平性
和 policy 仍有最终决定权。语言不能观察或依赖这个偏好。

## Service DB Independence

task capability 与 service DB capability 是两个独立 effect owner：

- task submission 不读取、加入、提交或回滚当前 DB transaction。
- DB transaction 内的普通 task submission 必须由用户可见语义明确禁止，不能伪装成与 DB 原子提交。
- TaskStore 即使物理上使用同一数据库产品，也不因此获得与业务 collection 的共享 transaction 语义。
- TaskStore / optional ready queue 的不同物理实现不改变 task dispatch 的 durable / at-least-once contract。

业务先写 DB、再提交 task 时可能在两者之间留下不一致；业务可以通过状态扫描、reconciliation 或显式 outbox 补救。outbox / CDC
若未来成为平台能力，必须作为一个明确的 DB-to-task bridge 单独设计：DB transaction 只原子写入 outbox intent，relay 再按
at-least-once 规则提交 task。普通 task dispatch 本身不获得隐式 dual-write 原子性。

## Observability And Retention

task observability 至少覆盖：

- submission accepted / rejected；
- scheduled -> ready latency；
- claim、attempt、Runtime selection 和 eligible wait；
- lease renew / loss / stale settlement；
- cancel succeeded / already-started / terminal / expired；
- target success / application failure / infrastructure recovery；
- permanent platform failure；
- duplicate notification absorbed；
- artifact retention blocked / unavailable；
- terminal age、backlog depth 和 oldest eligible age。

TaskId 是跨这些事件的主 correlation key；AttemptId 和 requestId 分别细化一次 claim 和一次 transport frame。日志和 telemetry
不能代替 TaskStore 的权威状态。

terminal record 保留期只服务状态查询与审计；terminal transition 已释放 execution artifact root。完整 payload 可以早于 terminal
metadata 删除。metadata 过期后，unknown TaskId ready notification 仍按非权威规则 drop，不能重建 task。

## Boundary With Other Mechanisms

- actor 负责可寻址实例、owner fencing 和同实例调用次序；task dispatch 负责持久接收、future eligibility 和 at-least-once
  attempt。actor-method task 同时经过两套 admission，但两套 lease 不合并。
- service DB 保存业务事实；TaskStore 保存平台调度事实。task outcome 不是业务结果日志。
- due-time index、scheduler 和可选 ready delivery 是 task dispatch 的底层机制，不是第二套用户可见 callable 语义。
- request-local `std.time.sleep` 只挂起当前 request，不能替代 future task。
- synchronous service / HTTP call 等待结果；detached task submission 只等待 durable acceptance，不等待 target outcome。
- recoverable-value contract 决定 payload 能否离开提交 request；task dispatch 不新增弱类型或 JSON fallback。

## Queue Exposure Boundary

第一版不向业务源码暴露 queue、partition、consumer、ack、lease 或 visibility。实现不保证内部一定存在 queue；若 adapter 使用
ready queue，它也只是 scheduler 的非权威 delivery lane。用户只持有 task reference，并通过 task status / cancel surface 操作
自己的 logical task。下列需求都不足以证明需要公开 queue：

- 立即与延迟 detached call；
- 削峰、fairness、并发限制和 backlog；
- task status、取消、基础设施恢复和 at-least-once attempt；
- actor-method future wake；
- Runtime replica 之间的位置透明派发。

这些需求仍能完整表述为“执行这个已知 callable”，暴露 queue 只会额外泄漏 claim、ack、lease、partition 和 store failure
模型。

只有 queue 本身成为业务地址和协议，而不是 callable 的实现细节时，才应设计独立用户 surface。至少出现下列一类需求才算
成立：

- producer 不知道或不应绑定具体 callable / consumer deployment，双方需要独立演进；
- 多个动态 consumer 竞争消费，consumer lifecycle 不由一个 target policy 静态决定；
- 业务需要显式 pull、batch、manual ack、visibility extension、pause / drain 或 dead-letter inspection；
- 外部系统需要直接生产或消费，而不是通过 Skiff callable API；
- message schema、retention、ordering / partition key 和访问控制本身是稳定业务 contract；
- backlog 需要在没有任何当前 handler build 的情况下独立存在，之后再绑定 consumer。

未来若出现这些需求，work queue 与 event stream 必须分开设计：

- work queue 表达一条 message 由一个竞争 consumer 处理，定义 ack / lease / redelivery。
- event stream / topic 表达多订阅者、retention、offset 和 replay；不能因底层都使用 log 就伪装成 work queue。

公开 queue item 不应偷偷携带 `ExecutableIdentity` 并退化成另一种 `dispatch`；它应拥有独立 schema identity，consumer binding
由 deployment / service contract 明确建立。反过来，`dispatch` 也不暴露 raw queue name 或 manual ack，仍保持 callable-first
语义。两者可以复用 TaskStore、scheduler 或物理 broker，但不能共享一套含混的用户 contract。

## Non-Goals

本文不提供：

- exactly-once callable effect；
- 业务 dedupe key 或相同 payload 合并；
- 任意 target return value、await result 或 durable workflow continuation；
- DB write 与 task submission 的隐式 distributed transaction；
- 取消已发生 effect 的 rollback；
- 严格准点 timer；
- 周期任务、cron、priority、automatic application-error retry 或 retry policy surface；
- 第一版公开 queue / topic，以及 queue / topic 作为业务事件日志；
- 物理 store、partition、index、polling 或 broker protocol 的固定实现。

## Canonical Contract Ownership

- `dispatch` 是唯一 detached-call source surface；旧 `spawn` 语法、易失 direct-spawn path 和旧 artifact branch 不属于目标态。
- detached task 的 canonical recoverable boundary 名为 `TaskDispatchPayload`。它继续遵守
  [`recoverable-value.md`](recoverable-value.md)，但新的 artifact / wire / runtime type 不得保留 `SpawnPayload` 双名。
- 本文的 TaskId + attempt + lease-loss retry 只属于 detached task。同步 HTTP / service request 即使物理上也经过 scheduler，
  不因此获得自动重试；同步 retry 继续服从 `../reference/runtime.md` 的独立 effect / idempotency policy。
- Runtime connection pending capacity 只计算 leased attempt 对应的 active request；scheduled / ready backlog 不计入任一 Runtime
  connection。
- timing clause、task reference 和 cancel result 的具体公开拼写属于 reference 设计，但都必须投影到本文唯一的 durable task
  dispatch contract。
- [`actor-model.md`](actor-model.md) 中的 detached Actor call 必须收敛到本文的 durable actor-method target：使用
  `ActorActivationSnapshot` 支持 registry 丢失后的 get-or-activate，并继续遵守旧 implementation rejection；不得保留独立易失
  `spawn` 队列。
- [`runtime-deployment-topology.md`](runtime-deployment-topology.md) 必须增加 retained task image 的 cold activation lane；它与
  active / draining generation pin 分开，不能为了未来 task 阻止旧 Runtime replica 正常退出，也不能在到期时解析 latest
  assembly 或 config snapshot。
