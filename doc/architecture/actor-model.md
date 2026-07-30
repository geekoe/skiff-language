# Actor Model

本文定义 Skiff actor 的目标态架构契约：定位边界、identity 与注册、常驻实例与协程并发、任期与 version、生命周期与恢复。本文只描述目标态；现状到目标态的迁移属于实现计划，不属于本文。

## 两个平面

平台把业务协调分成两个平面，actor 只负责其中一个：

- **数据面**：service-owned database。业务事实、长时工作、跨 version 共享的状态都在这里；单写者由数据库租约表达，后台推进由 `spawn` 唤醒。
- **内存面**：actor。可寻址、常驻、易失的内存对象，承载“短同步裁决”。

判定标准：

- 调用方的下一步动作在毫秒级依赖一个唯一裁决者的决定（操作序号、命中判定、成交回报、配额放行）→ actor。
- 工作时长超过毫秒级、结果必须持久、状态必须跨 version 一致 → 数据面。

按“收件箱在哪里”和“调用方等什么”分解，两个平面互补且不重叠：

| | 调用方等结果 | 调用方等“已接收” |
| --- | --- | --- |
| 内存收件箱 | actor 同步调用 | （不提供） |
| 持久收件箱 | （不提供；拆成“已接收 + 订阅结果”） | DB 写入 + `spawn` 唤醒 |

典型 actor 场景：协同编辑的操作排序、实时房间、在线 session、配额计数，以及对活跃 stream 的停止、替换和事件排序。完整 LLM 推理循环、长工具执行、聊天消息可靠接收仍属于数据面；其中正在运行的单个 model turn 可以同时用 actor 承载易失的实时控制状态。actor 不能取代 thread、message、run 和 checkpoint 的持久化模型。

## Actor 定义

actor 是显式声明的类型：声明携带 id 类型，字段是易失工作内存。

```skiff
actor DocHub id DocId {
  nextSeq: number
  pendingOps: Array<Op>
}

impl DocHub {
  function submitOp(self: DocHub, op: Op) -> SeqReceipt {
    const seq = self.nextSeq
    self.nextSeq = seq + 1
    self.pendingOps.push(op)
    return SeqReceipt { seq: seq }
  }
}
```

声明的具体语法形态在进入实现时由 reference 定稿；本文约束语义：

- actor 字段在实例存活期间跨调用保留；实例消亡即丢失。
- 需要跨实例存活的事实必须显式写 service-owned database。
- 一个 actor 类型有且只有一个 id 类型；id 必须可稳定 canonical 编码。声明形式让编译器在所有访问点强制这一点。
- actor 可以声明任意满足边界编码要求的成员方法。`stop`、`supersede` 等名字没有平台特殊语义，只是具体 actor 的普通方法。
- actor声明的具体名义类型就是外部句柄类型；例如`DocHub`变量只能调用`DocHub`成员方法，
  不能作为普通对象读取字段。成员调用与service function调用使用相同的类型化体验；router根据
  actor identity和incarnation epoch位置透明地路由到owner runtime。

## Identity 与注册

actor identity 由 service id、actor 类型、id 类型和 id 的 canonical 编码组成。service version / build id 不进入 identity：业务实体的地址必须跨发布稳定。

注册由 router control plane 维护：

- `getOrCreate(id, bootstrap)`：put-if-absent。entry已存在时返回现有句柄，不替换bootstrap、
  不打扰现有实例。这是常规入口；不用含义模糊的`ensure`。
- `replace(id, bootstrap)`：原子创建或替换entry。替换推进epoch并逐出现有实例；不用容易与
  普通值存储混淆的`put`。
- `find(id)`：存在则返回引用，不存在返回 `null`。
- `remove(id)`：删除 entry，逐出实例。

registry entry保存bootstrap值，只用于实例激活。registry不是持久层：router重启后entry丢失，
业务在入口路径用`getOrCreate`从业务事实重建。

## 常驻实例与协程并发

- 同一 identity 同时至多一个 live 实例，materialize 在单一 owner runtime 上。
- 实例在首个调用到达时从 bootstrap 激活。
- 不同 actor 实例可以由不同 executor 或线程并行执行；同一实例固定在一个单线程 actor executor 上，不允许多个 OS 线程同时访问它的字段。
- 同一实例的多个成员方法是并发协程。一个方法在同步代码段中独占 executor；stream next、异步 service call、WebSocket request、timer 等潜在 suspension point 只有在运行时实际等待时才释放执行权，此时其他方法可以执行。恢复后的方法必须假设 actor 字段已经变化。
- `connection.send` 只把消息同步写入本地发送队列，不等待网络或对端确认，因此不是 suspension point，也不提供送达或 exactly-once 保证。
- `std.websocket.requestJsonToConnection` 通过内置JSON-RPC 2.0 text配置发送request并等待匹配response；
  平台拥有且隐藏transport `id`。等待尚未完成时会释放执行权，因此是潜在suspension point。它只保证
  同一connection/generation内的transport配对，不提供业务幂等、自动重试或exactly-once。Ancestor
  cancellation终止该等待但不生成可捕获错误；deadline仍产生`TimeoutError`。
- 调用是同步的：调用方挂起等待返回。调用方所在 runtime 不需要拥有实例；路由是位置透明的。
- 实例状态的演化不写回 registry；逐出后重新激活回到 bootstrap。

没有 suspension point 的同步片段天然不会与同实例的其他方法交替执行，因此适合短同步裁决。runtime 不提供同实例字段的多线程共享内存语义，也不要求业务使用 mutex 或 atomic。没有 suspension point 的长同步方法会阻塞该实例的所有其他方法，直到返回、失败或被连续执行预算/watchdog终止；runtime 不在任意指令之间自动抢占，也不提供显式 `yield`。

compiler 的 `maySuspend` 是保守静态 summary，不是 runtime 调度指令。通过 `any I` / 未知 interface
dispatch 的调用即使被保守标记为可能挂起，也不会因此在调用前后自动释放 executor；若最终 concrete
执行没有遇到真实等待，该同步片段仍连续执行。service call本身是调用方的潜在挂起点，但也只有在
response尚未就绪、调用实际等待时才释放executor。callee实现内部的推断summary只可供其owner
runtime选择执行机制，不改变caller侧这一规则，也不属于ServiceContract。

长生命周期成员方法是合法的，只要它通过异步 IO、stream next 等真实等待周期性释放执行权。例如正在消费 LLM stream 的方法可以与 `stop` 并发：

```skiff
actor ActiveTurn id string {
  generation: number
}

impl ActiveTurn {
  function run(self: ActiveTurn, request: LlmRequest) -> void {
    self.generation += 1
    const generation = self.generation
    for delta in llm.stream(request) {
      if self.generation != generation {
        return
      }
      events.send(delta)
    }
  }

  function stop(self: ActiveTurn) -> void {
    self.generation += 1
  }
}
```

`stream.next()` 是潜在 suspension point：下一项已经缓冲时立即返回且不释放执行权，尚未到达时才挂起。因此 `stop` 可以在 `run` 实际等待 provider 时推进 generation。编译器仍将包含该操作的方法标记为 `maySuspend`，因为是否等待是运行时事实。generation 不是平台隐式插入的事务机制；它是业务在跨 suspension point 保持假设时使用的普通 actor 字段。没有跨 suspension point 的方法通常不需要 generation。

编译器可以诊断“方法在 suspension 前读取 actor 字段、恢复后继续依赖旧值”的明显模式，但不尝试证明并发程序正确。外部副作用跨 suspension point 时仍需由业务提供幂等、去重或补偿。

actor 的协程并发只隔离单个实例。actor 不是跨实体业务锁；跨实体一致性仍由数据库表达。

## 任期与 Version

- actor logical identity 不包含 service version。实例任期从激活开始，到逐出结束；任期内钉死单一 owner runtime、单一 implementation identity 和单一 epoch。
- compiler 为 actor 生成 ABI identity 和 implementation identity。ABI identity 覆盖 id 类型、字段布局与编码、公开成员方法签名和 actor runtime ABI；implementation identity 还覆盖规范化可执行 IR 及其可达依赖。
- identity 计算基于有限的声明、类型和调用依赖图，不递归展开类型。自引用和互相递归通过稳定符号引用与强连通分量 canonicalization 产生有限、确定的 fingerprint。
- fingerprint 相同只表示规范化编译结果相同；compiler 不尝试证明两个不同程序语义等价。
- service version 不同但 actor implementation identity 相同时，可以共同访问同一个 live incarnation；方法仍由该 incarnation 的 owner runtime 执行。
- actor implementation identity 不同时，不允许两个 incarnation 并发拥有同一 logical identity，也不迁移旧实例的内存字段。
- 需要跨 version 一致的数据不允许只存在 actor 内存里。

升级策略第一版固定，不提供逐 actor policy：

1. 第一个携带不同 implementation identity 的调用原子地把 live incarnation 标记为 `upgrading`，并指定目标 implementation。
2. `upgrading` 关闭新调用 admission，避免持续流量使旧实例永远无法退出。目标 implementation 的触发调用可以短暂等待；未在 deadline 内完成切换则收到可重试的 `ActorUpgradingError`。
3. 已执行的旧方法运行到最近一次真实挂起或正常返回；没有挂起点的长同步方法由连续执行预算和 watchdog 限制。runtime 在协程恢复前检查 incarnation 状态与 epoch；已被替换的方法以 `ActorIncarnationReplacedError` 结束。
4. active method 清零后销毁旧实例、推进 epoch，并在目标 version 的 runtime 上从 bootstrap 创建新实例。旧内存状态不保存、不复制。
5. 新 incarnation 激活后，只接受匹配其 implementation identity 的调用。后续旧 implementation 请求以 `ActorVersionRejectedError` 拒绝，不透明转发给新代码。

实现完全相同的旧 version 请求不属于升级，可以继续处理。实现不同但 ABI 恰好兼容的旧请求也不继续处理：结构可解码不代表业务语义兼容。若未来出现必须跨 implementation 延续任期的真实需求，再单独设计显式兼容与迁移机制，第一版不预留隐式推断。

“安全点”不是持久化 checkpoint。升级、runtime crash 或网络断开都可能发生在外部副作用完成而方法尚未返回的时刻；actor method 不获得 exactly-once 保证。

## 生命周期与恢复

- 没有 active method 且在配置的 idle TTL 内无人访问时实例为 idle；runtime 可以自动逐出 idle 实例。典型 TTL 是数分钟，不属于业务正确性承诺。
- 正常 idle 逐出只清理 live 内存，不删除 registry entry；下次调用从 bootstrap 重新激活。
- `upgrading` incarnation 即使有 suspended method 也会在这些方法到达安全点并退出后逐出。
- owner runtime 断连或 crash：排队与执行中的调用以平台错误返回调用方；实例状态丢失；下一个调用重新激活。
- 平台不持久化待执行调用队列。可靠投递、重试和补偿属于数据面。

## 边界规则

- actor声明产生的名义类型本身就是外部可持有的actor句柄类型；源码不存在额外的
  `ActorRef<T>`包装。例如`UserActor`既用于方法签名中的类型声明，也表示一个具体
  `UserActor`实例的可路由句柄。
- actor句柄只能用于调用actor方法：不能在外部读写actor字段，不能按普通值构造，不能写入DB，
  不能进入公开API payload。Runtime可以继续用内部`ActorRef`结构保存actor type、actor id和路由
  capability，但该结构不是Skiff源码类型。
- 方法参数与返回值必须可编码，不能携带 request-local handle。
- actor 字段只能由 owner actor executor 上的成员方法访问；后台 task 不得绕过 actor 调用直接持有可变字段引用。
- actor 不承担：持久状态容器、跨实体业务互斥、可靠消息投递。长工作可以是周期性 suspension 的成员方法，但其可靠事实、恢复点和最终结果仍必须进入数据面。
