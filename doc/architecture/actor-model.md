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

每次 actor get/create 或方法调用仍携带发起执行所要求的 exact deployment `buildId`。Runtime 以该
`buildId` 为唯一加载键，取得或懒加载 immutable `DeploymentExecutionImage` 后才进入 Actor admission；
该 invocation 及其 continuation pin 这个 execution owner，不从 ambient release pointer 重新选择代码或配置。
这里的“激活”专指 Actor 实例的 get/create 生命周期，不是 deployment activation；运行时不存在
`RuntimeAssembly`、current assembly 或 activation generation。

第一版注册入口只有 `std.actor.get`，它是 put-if-absent 的 get-or-create：

```skiff
let hub = std.actor.get<DocHub>("room-1", 0)
```

- entry 不存在：创建实例，由平台写入 key 字段，执行 `create`，激活，并把创建输入（id 与 create 参数）保存到 registry entry。
- entry 已存在：返回现有句柄，忽略本次 create 参数，不替换已有 entry、不打扰现有实例。创建参数只在 entry 缺失时生效；重新激活使用 entry 保存的创建输入，不使用调用点本次参数。
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

- actor 不在 live 时，task control plane 执行 **get-or-activate**：registry entry 存在时按
  entry 保存的创建输入激活实例；entry 因 Router 重启等原因丢失时，用 task 持久保存的
  `ActorActivationSnapshot` 恢复最小 entry（put-if-absent，首次恢复获胜）后执行 `create` 再调用
  目标方法。actor-method task 不保存 Actor 内存字段，也没有独立易失 `spawn` 队列。
- dispatched 调用与 caller request 生命周期分离；同一实例的多个 dispatched 调用按同步段串行，某次调用
  actual `Pending` 并释放 segment lease 后可以与其它调用交错。
- task lease 不替代 actor owner lease；task 执行仍遵守 actor 的 admission、升级与旧实现拒绝
  （`ActorVersionRejectedError` → 该 attempt 以 platform failure 收敛，不切回旧实现）。
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
  `ActorActivationSnapshot` 做同一 put-if-absent 恢复。两条路径都不恢复旧 Actor 内存字段。
- 实例状态的演化不写回 registry；idle 逐出后重新激活时，按 entry 保存的创建输入重新执行 create 构造初始状态，不恢复逐出前的内存状态。
- create 不要求是纯函数：允许挂起读取外部状态。典型实现是“记录存在则加载、不存在则按创建输入建立”——当 actor 需要持久状态时，create 内用 `db require` / `db find` 加载对应 db object 并回填成员，缺失则 `db insert` / `db upsert` 建立初始记录。重新激活因此以数据库当前事实为准，而不是恢复旧内存快照。

## 常驻实例与协程并发

- 同一 identity 同时至多一个 live 实例，materialize 在单一 owner runtime 上。
- 实例在首个调用到达时按 entry 创建输入激活：首次创建执行 create，之后从创建输入重建。
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

第一版不提供平台级业务打断：长方法只能运行到正常结束，或由实例生命周期机制（逐出、升级）终止；挂起点不会被平台强制终止（ancestor cancellation 除外）。`stop`、`cancel` 等名字没有平台特殊语义。业务需要取消时，用普通方法设置标记字段，业务逻辑在自身的检查点读取该字段协作退出。

runtime 在实例内部维护不暴露给业务的内部状态（如 incarnation epoch）。业务源码里不存在 generation 字段，也没有其它等价手工计数；跨 suspension point 的假设通过恢复后重新读取字段表达。

编译器可以诊断“方法在 suspension 前读取 actor 字段、恢复后继续依赖旧值”的明显模式，但不尝试证明并发程序正确。外部副作用跨 suspension point 时仍需由业务提供幂等、去重或补偿。

actor 的协程并发只隔离单个实例。actor 不是跨实体业务锁；跨实体一致性仍由数据库表达。

## 任期与 Version

- actor logical identity 不包含 service version 或 deployment `buildId`。实例任期从激活开始，到逐出结束；任期内钉死单一 owner runtime、单一 implementation identity 和单一 incarnation epoch。每个 method invocation / continuation 另外 pin 自己的 exact deployment `buildId` 与 immutable `DeploymentExecutionImage`。
- compiler 为 actor 生成 ABI identity 和 implementation identity。ABI identity 覆盖 key 字段类型与 canonical 编码、字段布局、公开成员方法签名和 actor runtime ABI；implementation identity 还覆盖规范化可执行 IR 及其可达依赖。
- identity 计算基于有限的声明、类型和调用依赖图，不递归展开类型。自引用和互相递归通过稳定符号引用与强连通分量 canonicalization 产生有限、确定的 fingerprint。
- fingerprint 相同只表示规范化编译结果相同；compiler 不尝试证明两个不同程序语义等价。
- service version / deployment build 不同但 actor implementation identity 相同时，可以共同访问同一个 live incarnation；方法仍由该 incarnation 的 owner runtime 执行，但每次 invocation 必须使用自身 pin 的 exact-build `DeploymentExecutionImage`，不能替换为 owner runtime 上的其它 build。
- actor implementation identity 不同时，不允许两个 incarnation 并发拥有同一 logical identity，也不迁移旧实例的内存字段。
- 需要跨 version 一致的数据不允许只存在 actor 内存里。

升级策略第一版固定，不提供逐 actor policy：

1. 第一个携带不同 implementation identity 的调用原子地把 live incarnation 标记为 `upgrading`，并指定目标 implementation。
2. `upgrading` 关闭新调用 admission，避免持续流量使旧实例永远无法退出。目标 implementation 的触发调用可以短暂等待；未在 deadline 内完成切换则收到可重试的 `ActorUpgradingError`。
3. 已执行的旧方法运行到最近一次 actual `Pending` 或正常返回；没有挂起点的长同步方法由连续执行预算和 watchdog 限制。runtime 在协程恢复前重新 acquire segment lease 并检查 incarnation fence 与 arena epoch；已被替换的方法以 `ActorIncarnationReplacedError` 结束。
4. active method 清零后销毁旧实例、推进 epoch，并在已加载或可懒加载目标 exact deployment `buildId` 的 runtime 上，按 entry 保存的创建输入执行目标 implementation 的 create 创建新实例。旧内存状态不保存、不复制。
5. 新 incarnation 激活后，只接受匹配其 implementation identity 的调用。后续旧 implementation 请求以 `ActorVersionRejectedError` 拒绝，不透明转发给新代码。

实现完全相同的旧 version 请求不属于升级，可以继续处理。实现不同但 ABI 恰好兼容的旧请求也不继续处理：结构可解码不代表业务语义兼容。若未来出现必须跨 implementation 延续任期的真实需求，再单独设计显式兼容与迁移机制，第一版不预留隐式推断。

“安全点”不是持久化 checkpoint。升级、runtime crash 或网络断开都可能发生在外部副作用完成而方法尚未返回的时刻；actor method 不获得 exactly-once 保证。

## 生命周期与恢复

- 逐出的安全条件是实例没有 active method（包括没有 suspended method）：有方法在执行或挂起时不能逐出，v1 没有平台级打断可以终止它。满足安全条件且 idle TTL 内无人访问时实例为 idle，runtime 可以自动逐出。安全条件回答“可以销毁”，TTL 回答“何时销毁”，避免逐出后立即重新激活的颠簸。典型 TTL 是数分钟，不属于业务正确性承诺。
- 正常 idle 逐出只清理 live 内存，不删除 registry entry；下次调用按 entry 保存的创建输入重新激活。
- `upgrading` incarnation 即使有 suspended method 也会在这些方法到达安全点并退出后逐出。
- owner runtime 断连或 crash：排队与执行中的调用以平台错误返回调用方；实例状态丢失；下一个调用按创建输入重新激活。
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
