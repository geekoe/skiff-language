# Actor Model

本文定义 Skiff actor 的目标态架构契约：定位边界、identity 与注册、常驻实例与串行执行、任期与 version、生命周期与恢复。本文只描述目标态；现状到目标态的迁移属于实现计划，不属于本文。

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

典型 actor 场景：协同编辑的操作排序、实时房间、撮合、配额计数。典型反例：LLM 推理循环、长工具执行、聊天消息接收——这些是数据面工作，不因为“存在一个业务对象的概念”就成为 actor。

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

## Identity 与注册

actor identity 由 service id、actor 类型、id 类型和 id 的 canonical 编码组成。service version / build id 不进入 identity：业务实体的地址必须跨发布稳定。

注册由 router control plane 维护：

- `ensure(id, bootstrap)`：put-if-absent。entry 已存在时返回现有引用，不替换、不打扰现有实例。这是常规入口。
- `put(id, bootstrap)`：原子创建或替换 entry。替换推进 epoch 并逐出现有实例。
- `find(id)`：存在则返回引用，不存在返回 `null`。
- `remove(id)`：删除 entry，逐出实例。

registry entry 保存 bootstrap 值，只用于实例激活。registry 不是持久层：router 重启后 entry 丢失，业务在入口路径用 `ensure` 从业务事实重建。

## 常驻实例与串行执行

- 同一 identity 同时至多一个 live 实例，materialize 在单一 owner runtime 上。
- 实例在首个调用到达时从 bootstrap 激活。
- 每个实例有一个 mailbox：调用按到达序排队，逐条 run-to-completion 执行。没有并发执行，没有 reentrancy，业务代码不需要锁。
- 调用是同步的：调用方挂起等待返回。调用方所在 runtime 不需要拥有实例；路由是位置透明的。
- 实例状态的演化不写回 registry；逐出后重新激活回到 bootstrap。

handler 纪律：

- handler 必须短：不做 LLM 调用、长 IO、长循环。平台以 deadline 强制。
- 长工作通过 `spawn` 外溢到数据面，handler 只做裁决和登记。
- 串行保证只覆盖单实例 mailbox。actor 不是跨实体业务锁；跨实体一致性仍由数据库表达。

## 任期与 Version

- 实例任期从激活开始，到逐出结束。任期内钉死单一 owner runtime 和单一 service version。
- 不同 version 不共享实例，也不并发拥有同一 identity。
- version 交接发生在任期边界：旧实例逐出后，由新 version 重新激活。
- 需要跨 version 一致的数据不允许只存在 actor 内存里。

## 生命周期与恢复

- mailbox 为空且无执行中调用时实例为 idle；只有 idle 实例可以被逐出。逐出清理内存，不删除 registry entry。
- owner runtime 断连或 crash：排队与执行中的调用以平台错误返回调用方；实例状态丢失；下一个调用重新激活。
- 平台不持久化 mailbox。可靠投递、重试和补偿属于数据面。

## 边界规则

- `ActorRef` 只能用于调用 actor 方法：不能读写字段，不能写入 DB，不能进入公开 API payload，不能手写构造。
- 方法参数与返回值必须可编码，不能携带 request-local handle。
- actor 不承担：持久状态容器、业务互斥锁、长工作宿主、可靠消息投递。
