# Actor Model

本文定义 Skiff actor 的目标态架构契约：定位边界、identity 与注册、常驻实例与协程并发、任期与 version、生命周期与恢复。本文只描述目标态；现状到目标态的迁移属于实现计划，不属于本文。

## 两个平面

平台把业务协调分成两个互补机制：

- **数据面**：service-owned database。业务事实、长时工作、跨 version 共享的状态都在这里；同一实体的运行唯一性由 actor 面表达，后台推进由 `dispatch` 唤醒。
- **actor 面**：运行时唯一实体。同一 identity 同时至多一个 live 实例，成员方法的同步段在同一实例上串行执行，只有 actual `Pending` 释放 segment lease 后协程才会交错，承载“短同步裁决”。actor 不承诺持久性，但持久性不是它的定义特征；定义特征是运行时唯一性。

唯一性归属：业务声明的 `db object` 不承载租约语义或语法；“谁在推进、谁能写”的运行时唯一性由
actor 面承担。平台内部可以使用租约实现 actor 的单实例保证与 fencing，但那是实现机制，不作为
业务 API，也不以 `lease` 声明形式长在 `db object` 上。

判定标准：

- 调用方的下一步动作在毫秒级依赖一个唯一裁决者的决定（操作序号、命中判定、成交回报、配额放行）→ actor。
- 工作时长超过毫秒级、结果必须持久、状态必须跨 version 一致 → 数据面。

按“收件箱在哪里”和“调用方等什么”分解，两个机制互补且不互相替代：

| | 调用方等结果 | 调用方等“已接收” |
| --- | --- | --- |
| 内存收件箱 | actor 同步调用 | （不提供） |
| 持久收件箱 | （不提供；拆成“已接收 + 订阅结果”） | DB 写入 + `dispatch` 唤醒 |

典型 actor 场景：协同编辑的操作排序、实时房间、在线 session、配额计数，以及对活跃 stream 的停止、替换和事件排序。完整 LLM 推理循环、长工具执行、聊天消息可靠接收仍属于数据面；agent 场景中 thread 本身是 actor，正在运行的 model turn 是它的活跃方法。actor 不能取代 thread、message、run 和 checkpoint 的持久化模型。

## Actor 定义

actor 是 `type` 的包裹与限制：`type` 声明字段 shape，`actor` 声明把同一个名字包裹成 actor 句柄，`impl` 提供这个包裹内的行为。actor 声明不创建第二个源码类型名，也不携带字段；字段只来自 attached type。actor 的特征是运行时唯一性，不是易失性；v1 不允许同一个 type 同时挂 `db object` 和 `actor`（禁止双挂），持久状态通过独立 type 和显式 db 操作表达。

```skiff
type DocHub {
  id: DocId
  nextSeq: number
  pendingOps: Array<Op>
}

actor DocHub {
  key(id)
  create(initialNextSeq: number)
}

impl DocHub {
  function create(self: DocHub, initialNextSeq: number) -> void {
    self.nextSeq = initialNextSeq
  }

  function submitOp(self: DocHub, op: Op) -> SeqReceipt {
    let seq = self.nextSeq
    self.nextSeq = seq + 1
    self.pendingOps.push(op)
    return SeqReceipt { seq: seq }
  }
}
```

规则：

- `actor X` 必须附着到同文件同名 `type X`；attached type 必须是非泛型 concrete record。`X` 既是字段 shape 的声明者，也是外部可持有的 actor 句柄类型。
- v1 禁止双挂：`actor X` 与 `db object X` 不能附着到同一个 type。持久事实通过独立 type 表达（可以出现在 actor 成员中），actor type 本身不声明任何存储元数据。
- actor 字段是易失工作内存，不自动持久化、不自动回填；持久化完全通过 impl 中的显式 db 操作表达（典型模式：create 里 `db require` / `db insert` 加载或建立，方法里 `db update` 写回）。actor type 不区分 volatile / 持久字段；成员可以是任意可编码类型，包括带 `db object` 附着的 record 类型，成员本身不产生持久化语义。一个 actor 可以拥有任意多个这类成员。
- 推荐形态（执行器模式）：actor 类型可以只含 key 字段（例如 `ThreadActor`），持久事实放在独立 db object 类型（例如 `Thread`）中，impl 方法通过显式 db 操作访问。actor 类型与持久类型不必同名、不需要包裹关系；`actor` 声明仍然必须存在（它提供 key/create 与句柄语义），仅当 attached type 除 key 外没有其他字段时可省略 create。
- actor 声明只描述 actor 面元数据：`key(field)` 和可选的 `create(...)`。成员方法不进 actor 声明，全部通过 `impl` 引入。
- `create` 是平台保留方法：由平台在每个新 incarnation 的激活路径调用，不进句柄 method namespace，业务代码不能通过句柄直接调用。
- `key(field)` 指定 identity 字段：必须是 attached type 的字段，类型必须可稳定 canonical 编码。key 字段由平台在激活时写入（create 执行前），成员方法内只读。key 与持久状态主键的对应关系由 impl 维护，v1 编译器不强制。
- actor 字段在实例存活期间跨调用保留；实例消亡即丢失。
- 需要跨实例存活的事实必须显式写 service-owned database。
- actor 可以拥有任意满足边界编码要求的成员方法。`stop`、`supersede` 等名字没有平台特殊语义，只是具体 actor 的普通方法；平台操作走显式 intrinsic。
- actor 的名义类型就是外部句柄类型：`DocHub` 变量只能调用 `DocHub` 成员方法，不能作为普通对象读取字段。成员调用与 service function 调用使用相同的类型化体验；router 根据 actor identity 和 incarnation epoch 位置透明地路由到 owner runtime。

## Identity 与注册

actor identity 由 service id、actor 类型、key 字段类型和 key 的 canonical 编码组成。service version / build id 不进入 identity：业务实体的地址必须跨发布稳定。

每次 actor get/create 或方法调用都携带发起执行所要求的 exact deployment `buildId`。这个 build 不进入
logical identity，而是该请求希望访问的 Actor 代码版本。版本选择是**逐 Actor identity**的：不存在 service
级 Actor release pointer，也不要求同一 Actor type 的所有 key 同时切版；`DocHub("a")` 可以运行 build A，
同时 `DocHub("b")` 运行 build B。

一个 live incarnation 从 create 到销毁始终 pin **同一个** exact `buildId`、immutable
`DeploymentExecutionImage`、implementation identity 和 owner runtime；其所有已 admission 的 method 与
continuation 都在 owner image 中执行。调用方自己的 image 只负责在调用边界生成/验证参数和结果，不能把
caller image-local type/shape/const/method-table 索引直接带进 ActorStateHeap。请求的 build 与 live
incarnation build 不同时，不在旧 heap 上执行新代码，直接按下文规则拒绝。

这里的“激活”专指 Actor 实例的 get/create 生命周期，不是 deployment activation；运行时不存在
`RuntimeAssembly`、current assembly、activation generation 或 Actor 专用 release pointer。

第一版注册入口只有 `std.actor.get`，它是按 logical identity / live owner fence 串行化的 get-or-create：

```skiff
let hub = std.actor.get<DocHub>("room-1", 0)
```

- entry 不存在：创建实例，由平台写入 key 字段，执行 `create`，激活，并把创建输入（id 与 create 参数）保存到 registry entry。
- live incarnation 已存在且 build 相同：返回现有句柄，忽略本次 create 参数，不打扰现有实例。
- live incarnation 已存在但 build 不同：拒绝本次 get，不更新 entry，不影响旧实例的 idle/lease 时钟。
- 没有 live incarnation：本次 get 可以竞争新的 owner claim；获胜者把自己的 exact build、ABI 与 create
  输入绑定到新 incarnation。旧 entry 中保存的 create 输入只有在 expected plan 与本次 build 精确兼容时
  才能复用；否则使用本次 get 的完整 create 输入，缺失则 fail closed。
- `create` 可以省略：仅当 attached type 除 key 字段外没有其他字段时合法（v1 没有字段默认值）。省略后 `get<T>(id)` 直接激活。
- 声明了 `create` 时，`get<T>(id, ...createArgs)` 的调用形态由声明中的 create 签名合成；编译器在所有调用点强制参数一致。
- `replace`、`find`、`remove` 等注册控制操作不在第一版，出现真实需求后再定义；实例回收由 idle TTL 表达。

`create` 是初始化方法，不是外部成员：

- 不能通过句柄调用；只在平台为新 incarnation 执行的 Actor 激活路径中执行。
- actor 声明中的 create 签名与 `impl` 中 create 方法的签名（不含 self 的参数表与返回类型）必须精确一致；重复、缺失、不一致都是编译错误。
- create 返回前必须对所有非 key 字段完成赋值（definite assignment）；未赋值就读取或返回报编译错误。
- create 可以包含潜在 suspension point（典型是 `db require` / `db insert` / `db upsert`）。create 执行期间实例未 admission：其他方法不能进入，调用方的 `get` 等待 create 返回后才拿到句柄；因此挂起不会产生半初始化可见性或并发交错。
- create 内不能调用本实例其他方法。
- key 字段在 create 内只读；平台在 create 执行前写入。

### dispatch 到 actor 方法（自消息 / 异步推进）

`dispatch` 可以以 actor 方法为 target：提交的调用是 durable actor-method task（权威契约：
[`durable-task-dispatch.md`](durable-task-dispatch.md) "Actor-method target"）。提交时同时冻结 exact
deployment `buildId` 和 `ActorActivationSnapshot`（key / create 输入 / expected-type plan），TaskStore
接受后作为该 actor 的一次独立方法调用；Runtime 只从该 `buildId` 的 immutable
`DeploymentExecutionImage` 执行目标方法，按 identity 路由并服从同一 instance 的 segment lease；调用方不等结果
（fire-and-forget，只等 durable “已接收”）。

- actor 不在 live 时，task control plane 执行 **get-or-activate**：用 task 持久保存的
  `ActorActivationSnapshot` 参与 owner claim；首次成功的 exact build 执行 `create` 再调用目标方法。
  compatible registry create input 可以复用，但 registry 中历史 implementation 不是版本指针。
  actor-method task 不保存 Actor 内存字段，也没有独立易失 `spawn` 队列。
- dispatched 调用与 caller request 生命周期分离；同一实例的多个 dispatched 调用按同步段串行，某次调用
  actual `Pending` 并释放 segment lease 后可以与其它调用交错。
- task lease 不替代 actor owner lease；task 执行仍遵守同一个逐 identity build admission。task 冻结的
  build 与 live incarnation 不同时，不能把旧 payload 交给另一 build 的代码；该 attempt 以
  `ActorVersionRejectedError` 收敛。Actor 已销毁时，后续独立请求仍可用任意可加载 build 重新创建。
- 这是业务表达“继续推进 / 给自己发消息”的通用原语：例如消息处理完成后
  `dispatch actor.tick(...)`，tick 作为普通方法在之后执行，不嵌套在投递方法调用栈内。
- 平台不提供独立的 `wake` 保留原语；需要唤醒时直接用 `dispatch` 目标方法。

### 消费视图与验收矩阵

actor 声明的权威表示是 PackageArtifact 中的 actor 元数据（key/create/方法/identity）。
编译器、runtime linker、runtime 执行、router 与测试 harness 都从该表示消费，不得各自
重新推导或保留第二套形态。已知消费视图：

1. 生产同包路径（`root.*` 直接调用 actor 方法）——compile / link / execute；
2. 公共 API 跨包调用方（public view）——compile / link / execute；
3. `kind: test` service 经 `topLevelAlias` 的测试视图——compile / link / execute；
4. router 控制面（`get` / `dispatch` / owner 路由）——运行期。

验收红线：actor 相关改动必须覆盖视图 1、2、3 的 compile/link/execute 与视图 4 的运行期
路径，否则视为未闭合。历史缺口示例：视图 3 的 runtime link 曾报
`type symbol ... is unresolved`（编译器投影已做、linker 消费缺失）；spawn 到 actor 的
异常/trace 上下文曾未穿透 invoke 帧。二者均为“消费视图未枚举、单一事实源未落实”的
表现，修复时须同时补对应视图的回归。

registry entry 保存创建输入，不保存实例状态，也不是 deployment activation state：

- entry 是 Actor 激活所需的最小易失事实，不是持久层；Router 重启、进程丢失或 operator 清理都可能使其
  丢失。普通入口随后用 `get` 的 id / create 参数从业务事实重建；durable actor-method task 则使用自身冻结的
  `ActorActivationSnapshot` 竞争恢复。两条路径都不恢复旧 Actor 内存字段。
- entry 中与 build/ABI/expected plan 绑定的 activation snapshot 只服务于当前 live incarnation，不能在实例
  销毁后继续充当版本 fence。没有 live owner 时，首个 claim 可以用自身 exact build 与兼容的保存输入重建，
  或以自身完整 create 输入原子替换不兼容的 snapshot；并发 claim 仍只有一个获胜。
- 实例状态的演化不写回 registry；idle 逐出后重新激活会再次执行 create 构造初始状态，不恢复逐出前的内存状态。
- create 不要求是纯函数：允许挂起读取外部状态。典型实现是“记录存在则加载、不存在则按创建输入建立”——当 actor 需要持久状态时，create 内用 `db require` / `db find` 加载对应 db object 并回填成员，缺失则 `db insert` / `db upsert` 建立初始记录。重新激活因此以数据库当前事实为准，而不是恢复旧内存快照。

## 常驻实例与协程并发

- 同一 identity 同时至多一个 live 实例，materialize 在单一 owner runtime 上。
- 实例在没有 live owner 时由首个成功 claim 的 exact build 激活；create 输入来自该请求的完整 snapshot，或
  来自与其 expected plan 精确兼容的 registry snapshot。
- 不同 actor 实例可以由不同 executor 或线程并行执行；同一实例由逻辑 Actor executor 与 segment lease
  串行化，不要求硬 OS-thread affinity，但任何时刻不允许多个 OS 线程同时访问它的字段。
- 每个 live 实例拥有一个 instance-owned shared arena 和稳定 field root；同一实例的所有方法协程直接共享这份 state，不把字段克隆到逐方法 request heap，也没有 return-time commit overlay。
- 每个同步段持有 `ActorSegmentLease`，直接读写 shared arena。已经执行的字段 / 节点写入立即对后来取得 lease 的协程可见，包括本方法实际返回 `Pending` 前的写入；普通 return 或失败都不提交副本，也不回滚已执行写入。
- 同一实例的多个成员方法是并发协程。stream next、异步 service call、WebSocket request、timer 等只是潜在 suspension point；只有本次操作实际返回 `Pending` 时才释放 segment lease 和执行权。同步 ready 不产生 yield，静态 `maySuspend` 也不产生预先释放。
- continuation 恢复前必须重新 acquire `ActorSegmentLease`，并重新校验 actor identity、exact deployment
  `buildId` / `DeploymentExecutionImage` owner、`ActorImplementationIdentity`、incarnation fence 与 arena epoch；
  任何不匹配都 fail closed。恢复后的方法必须假设 actor 字段已经变化并按需重新读取。
- 普通 record / Array / Map 仍采用 value semantics：赋值、普通参数传递、返回与 container store 产生 logical snapshot。局部 `let` 和普通参数不可写；局部 `var` 是 writable binding，首次写共享 backing 时按 path COW 分离。把 `self.field` 读入普通 local 或传给普通参数只得到 snapshot，不获得隐藏的 mutable alias；直接 writable `self.field` path 仍然修改 Actor shared state。
- `connection.send` 只把消息同步写入本地发送队列，不等待网络或对端确认，因此不是 suspension point，也不提供送达或 exactly-once 保证。
- `std.websocket.requestJsonToConnection` 通过内置JSON-RPC 2.0 text配置发送request并等待匹配response；
  平台拥有且隐藏transport `id`。等待尚未完成时会释放执行权，因此是潜在suspension point。它只保证
  同一connection/generation内的transport配对，不提供业务幂等、自动重试或exactly-once。Ancestor
  cancellation终止该等待但不生成可捕获错误；deadline仍产生`TimeoutError`。
- 调用是同步的：调用方挂起等待返回。调用方所在 runtime 不需要拥有实例；路由是位置透明的。

没有 suspension point 的同步片段天然不会与同实例的其他方法交替执行，因此适合短同步裁决。runtime 不提供同实例字段的多线程共享内存语义，也不要求业务使用 mutex 或 atomic。没有 suspension point 的长同步方法会阻塞该实例的所有其他方法，直到返回、失败或被连续执行预算/watchdog终止；失败只结束调用，不把本段已经完成的 Actor 写入回滚。runtime 不在任意指令之间自动抢占，也不提供显式 `yield`。

compiler 的 `maySuspend` 是保守静态 summary，不是 runtime 调度指令。通过 `any I` / 未知 interface
dispatch 的调用即使被保守标记为可能挂起，也不会因此在调用前后自动释放 executor；若最终 concrete
执行没有遇到真实等待，该同步片段仍连续执行。service call本身是调用方的潜在挂起点，但也只有在
response尚未就绪、调用实际等待时才释放executor。callee实现内部的推断summary只可供其owner
runtime选择执行机制，不改变caller侧这一规则，也不属于ServiceContract。

长生命周期成员方法是合法的，只要它通过异步 IO、stream next 等真实等待周期性释放执行权。方法之间可以交错，业务不维护 generation 字段。在 agent 场景中，这个 actor 就是 thread：

```skiff
type Thread {
  id: string
  running: bool
}

actor Thread {
  key(id)
  create()
}

impl Thread {
  function create(self: Thread) -> void {
    self.running = false
  }

  function run(self: Thread, request: LlmRequest) -> void {
    self.running = true
    for delta in llm.stream(request) {
      events.send(delta)
    }
    self.running = false
  }

  function isRunning(self: Thread) -> bool {
    return self.running
  }
}
```

`stream.next()` 是潜在 suspension point：下一项已经缓冲时立即返回且不释放执行权，尚未到达时才挂起。因此 `run` 在等待 provider 时，同实例的其它方法（如 `isRunning`）可以执行。编译器仍将包含该操作的方法标记为 `maySuspend`，因为是否等待是运行时事实。

第一版不提供平台级业务打断：长方法只能运行到正常结束，或由实例逐出/owner失效等生命周期机制终止；挂起点不会仅因出现不同 build 的请求而被平台强制终止（ancestor cancellation 除外）。`stop`、`cancel` 等名字没有平台特殊语义。业务需要取消时，用普通方法设置标记字段，业务逻辑在自身的检查点读取该字段协作退出。

runtime 在实例内部维护不暴露给业务的内部状态（如 incarnation epoch）。业务源码里不存在 generation 字段，也没有其它等价手工计数；跨 suspension point 的假设通过恢复后重新读取字段表达。

编译器可以诊断“方法在 suspension 前读取 actor 字段、恢复后继续依赖旧值”的明显模式，但不尝试证明并发程序正确。外部副作用跨 suspension point 时仍需由业务提供幂等、去重或补偿。

actor 的协程并发只隔离单个实例。actor 不是跨实体业务锁；跨实体一致性仍由数据库表达。

## 任期与 Version

- actor logical identity 不包含 service version 或 deployment `buildId`。实例任期从激活开始，到逐出结束；
  任期内钉死单一 owner runtime、单一 exact deployment `buildId` / immutable
  `DeploymentExecutionImage`、单一 implementation identity 和单一 incarnation epoch。不同 logical
  identity 的任期互相独立，可以同时钉住不同 build。
- compiler 为 actor 生成 ABI identity 和 implementation identity。ABI identity 覆盖 key 字段类型与
  canonical 编码、字段布局、`create`参数与编码、公开成员方法签名和 actor runtime ABI；implementation
  identity 还覆盖规范化可执行 IR、const/code identity及其可达依赖。
- identity 计算基于有限的声明、类型和调用依赖图，不递归展开类型。自引用和互相递归通过稳定符号引用与强连通分量 canonicalization 产生有限、确定的 fingerprint。
- fingerprint 相同只表示规范化编译结果相同；compiler 不尝试证明两个不同程序语义等价。
- 第一版即使两个 deployment 的 actor ABI/implementation identity 完全相同，只要请求 build 与 live
  incarnation build 不同，也不能共同访问同一个 live heap；只有现有instance经普通生命周期销毁后，另一
  build才可能赢得下个incarnation。这样
  ActorStateHeap 内的 image-local tag/shape/const/method-table 始终由任期 pin 的单一 image 解释。
- 任意时刻不允许两个 deployment incarnation 并发拥有同一 logical identity，也不把旧实例内存字段迁移到
  新实例。
- 需要跨 version 一致的数据不允许只存在 actor 内存里。

版本 admission 第一版固定，不升级 live Actor，不记录 current/newest build，也不比较 semver：

1. 请求 build 与该 identity 的 live incarnation exact build 相同，才可以 ordinary admission，并刷新该
   incarnation 的 owner lease / idle 时钟。
2. build 不同的 get、method 或 durable task 直接以 `ActorVersionRejectedError` 拒绝；不进入当前 heap，
   不更新 registry/version facts，不触发 retirement，也不刷新旧 incarnation 的 lease / idle 时钟。
3. 同 build 已经 admission 的 method/continuation 继续由 owner image 执行，直到正常结束、失败或实例按普通
   生命周期被回收；发布新 build 本身不打断它们。
4. idle TTL、owner runtime 断连、shutdown 等普通生命周期事件销毁 live instance后，释放 owner image pin、
   清除该 incarnation 的 build admission facts并推进 epoch。旧内存状态不保存、不复制。
5. 此后第一个成功取得该 identity owner claim、且携带完整或兼容 create snapshot 的请求，用**自己的** exact
   build 重新执行 `create`。平台不记住哪个 build 是“新版”，所以允许向新 build 前进，也允许旧 build
   重新获胜而回退。
6. 两个 build 并发竞争空 identity 时只允许一个 claim/owner fence 成功；失败方重新观察结果，并在 build
   不同的情况下按第 2 条拒绝，不能并发创建第二个实例。

这个简单规则的明确代价是：只要同 build 流量持续刷新 idle 时钟，hot Actor 可以长期停留在旧 build；部署
系统不能等待 runtime 主动升级它。Actor runtime 也不保存 release pointer、superseded-build 集合或临时升级
target。未来可以把 `ActorAbiIdentity` 相同作为跨 build 复用 live heap 的必要条件，并显式重绑定所有
image-local tag/shape/const/behavior 引用；这只是优化。第一版不做该优化：ABI 相同仍拒绝跨 build 调用，
ABI 不同也不能读取旧 heap 或把旧编码直接交给新 image。

“安全点”不是持久化 checkpoint。逐出、runtime crash 或网络断开都可能发生在外部副作用完成而方法尚未返回的时刻；actor method 不获得 exactly-once 保证。

## 生命周期与恢复

- runtime 会根据 idle TTL 自动发起逐出；TTL/扫描周期是operator/runtime policy，不是业务可依赖的
  定时承诺。达到TTL后先关闭新admission并把live instance标为待销毁；已在运行的同步段只在
  回到runtime检查点时观察fence，suspended continuation在resume前观察fence。只有active/suspended/cleanup
  计数全部归零后才物理释放heap。这是普通实例生命周期，与是否出现新build请求无关。
- Router 不能仅因owner lease本地过期就把实例当成已销毁并开放新owner。正常idle路径要求
  exact-owner discard ACK；owner断连/crash路径要求session/incarnation fence使旧owner不再能admit/resume，
  新incarnation才能发布。否则会在Router无owner而Runtime仍有残留instance时破坏唯一性。
- 正常 idle 逐出清理 live 内存和该 incarnation 的 build admission facts；registry 可以保留 key/create
  输入作为重建材料，但它不是版本指针。下次激活由首个 claimant 的 exact build 决定；保存输入与该 build
  的 create plan 不兼容时必须要求该请求提供新的完整 create 输入，否则 fail closed。
- 不同 build 的拒绝不改变当前实例；只有 idle TTL、owner/runtime 生命周期与显式平台清理会触发逐出。
- owner runtime 断连或 crash：排队与执行中的调用以平台错误返回调用方；实例状态丢失；下一个成功 claimant
  按自己的 exact build 与 create snapshot 重新激活。
- actor 面本身不持久化待执行调用队列；actor-method `dispatch` 的可靠投递、基础设施恢复与
  at-least-once attempt 由 durable task dispatch 承载（见
  [`durable-task-dispatch.md`](durable-task-dispatch.md)），业务补偿属于数据面。
- 身份删除（移除 registry entry 并清理持久状态）不在 v1 提供，需要显式操作：quiescence 只是必要条件——句柄仍可能被持有、dispatched call 可能还在路上、持久状态需要业务清理，这些都不能从实例状态推导。

## 边界规则

- actor 声明把 attached type 的名字包裹成外部可持有的 actor 句柄类型；源码不存在额外的
  `ActorRef<T>`包装。例如`UserActor`既用于方法签名中的类型声明，也表示一个具体
  `UserActor`实例的可路由句柄。
- actor 句柄只能用于调用 actor 方法：外部代码获得句柄后不能访问成员变量（key 字段同样只读），不能按普通值构造，不能写入 DB，
  不能进入公开 API payload。Runtime 可以继续用内部 `ActorRef` 结构保存 actor type、actor id 和路由
  capability，但该结构不是 Skiff 源码类型。
- 方法参数与返回值必须可编码，不能携带 request-local handle。
- actor 字段只能由 owner actor executor 上的成员方法访问；后台 task 不得绕过 actor 调用直接持有可变字段引用。
- actor 的 `impl` 必须与 actor 声明同文件：成员方法集合是 actor ABI 的一部分，跨文件 impl 会让 ABI 扫描面扩散；普通 type 的 impl 不受此限制。
- `create` 不进入句柄的 method namespace，只由平台激活路径调用。
- actor 不承担：持久状态容器、跨实体业务互斥、可靠消息投递。长工作可以是周期性 suspension 的成员方法，但其可靠事实、恢复点和最终结果仍必须进入数据面。
