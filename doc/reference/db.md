# Skiff DB Reference

本文负责：稳定描述 Skiff service-owned database 的用户可见语言规则，包括 `db object`、读写操作、query block、projection、返回类型、transaction、lease、数据库归属和当前不支持事项。

本文不负责：compiler lowering、runtime Mongo adapter、artifact DTO、索引 rollout、schema migration、跨 service 数据复制、actor / queue / timer 调度和实现迁移计划。

## 1. Data Model

Skiff DB 是 service-owned object database 能力。业务代码面向 typed object、typed query 和显式写入操作，不面向 collection 字符串、Mongo filter 或 Mongo update operator。

`type` 是对象 shape 的唯一源码类型声明。一个可持久化对象必须先有同模块同名 record type：

```skiff
type User {
  id: string
  name: string
  visits: number
  createdAt: number
}

db object User {
  name "user"
  primary key(id)
  index byCreated(createdAt desc)
}
```

`db object User` 是对 `type User` 的数据库附着声明。它不创建第二个源码类型名，不是 `User` 的 alias，也不是 Mongo collection 的薄包装。

规则：

- `db object` 必须附着到同模块同名 `type`。
- attached type 当前必须是非泛型 concrete record type。
- stored fields 来自 attached `type` 的 record fields。
- 每个 `db object` 必须声明单字段 primary key。
- primary key 必须是 attached type 上的 stored field。
- 用户不能声明 `_id` 字段，底层 `_id` 只由 runtime adapter 从 key 字段映射。
- 默认 collection name 使用 object 名称，显式 `name` 只用于物理存储映射。
- `db object` 不生成 `Row`、`Document`、`Entity` 等额外源码名字。

## 2. Field Paths And Contextual Keywords

DB block 内的 `fields`、`where`、`order`、`limit`、`offset`、`unset`、`add`、`remove` 等只作为上下文关键字。它们不是全局保留字段名。

字段列表必须有显式边界：

```skiff
db find many User {
  fields { name, visits }
  where createdAt > 1
}
```

这避免了旧形态的歧义：

```skiff
// 不再作为 canonical syntax：
db find many User { fields name visits where createdAt > 1 }
```

如果对象字段名就是 `where`，写在 `fields { ... }` 中：

```skiff
db find many User {
  fields { where, name }
  where createdAt > 1
}
```

`fields { ... }` 中的 entry 是 DB field path，允许 top-level field 和已支持的 nested stored field path。query、order、change 和 projection 的 field path 都必须能从当前 target object 的 stored field graph 静态验证。

Nested projection 只穿过静态可验证的 stored record shape。`Json`、array、map、union 或未来动态对象字段不提供可投影的子字段，除非对应能力另行定义。穿过 nullable record 时，projection 保留 nullable 边界。
使用 recoverable-envelope lane 的 stored field 第一版也不可穿透：可以选择整个 top-level 字段，但不能对其内部
field path 做 projection、predicate、order 或 index。

```skiff
type UserProfile {
  displayName: string
  avatar: { url: string, width: number }
}

type User {
  id: string
  profile: UserProfile?
}

const users = db find many User {
  fields { profile.displayName, profile.avatar.url }
}
```

上例元素类型是：

```skiff
{ id: string, profile: { displayName: string, avatar: { url: string } }? }
```

同一个 projection 不能同时选择父路径和子路径，例如 `fields { profile, profile.displayName }`。需要完整字段时选择父路径；需要部分字段时只选择子路径。

## 3. Read Operations

读取能力包括：

- `db find Target(key)`：按 key 读取，缺失返回 `null`。
- `db optional Target(key)`：按 key 读取，缺失返回 `null`。
- `db require Target(key)`：按 key 读取，缺失抛出 not-found 类错误。
- `db find Target { ... }`：按 query 读取一个对象，缺失返回 `null`。
- `db optional Target { ... }`：按 query 读取一个对象，缺失返回 `null`。它与 query 形态的 `find` 返回类型相同，用于表达调用点希望强调 nullable 语义。
- `db require Target { ... }`：按 query 读取一个对象，缺失抛出 not-found 类错误。
- `db find many Target { ... }`：按 query 读取对象数组。
- `db count Target { ... }`：返回匹配数量。
- `db exists Target(key)` 或 `db exists Target { ... }`：返回是否存在。

Key read 可以追加只含 projection 的 block：

```skiff
const user = db require User(id) {
  fields { profile.displayName }
}
```

这个 block 只能包含 `fields { ... }`。`where`、`order`、`limit`、`offset`、`after` 和 `load` 不属于 key read block；需要按条件读取时使用 query read 形态。

没有 `fields` projection 时，read 返回 key 加全部 stored fields 的 full snapshot。

有 `fields { ... }` projection 时，read 返回 key 加所选 stored fields。primary key 总是自动包含在 read projection 中，即使源码没有列出。

示例：

```skiff
const users = db find many User {
  fields { name, visits }
  where createdAt > 1
  order id asc
  limit 20
}
```

上例的元素类型是匿名 record：

```skiff
{ id: string, name: string, visits: number }
```

`createdAt` 只参与 query predicate，不出现在返回类型中。

读取结果是 readonly snapshot / projection。字段赋值不会写回数据库；需要修改持久状态时必须使用显式 DB write operation。

## 4. Query Block

query block 可以包含 projection、predicate、order 和分页选项：

```skiff
db find many User {
  fields { name, visits }
  where visits > 0
  where createdAt > 1
  order id asc
  offset 20
  limit 20
}
```

规则：

- 多个 `where` 按 AND 组合。
- OR / NOT 使用普通 boolean 表达式。
- `where if condition { predicate }` 可以条件式加入 predicate，条件本身不能引用 DB 字段。
- `order` 顺序有语义，runtime 不自动追加隐藏排序字段。
- 分页当前只支持 `offset` 和 `limit`。
- `after` / cursor / continuation 不属于当前 DB surface。
- query block 不是 JSON / Mongo query object。

## 5. Write Operations

写入 operation 必须显式表达业务意图：

- `db insert Target { ... }` 创建单个对象，必须提供 key 和所有必填 stored fields。
- `db insert many Target values rows` 创建多个对象，返回插入计数。
- `db update Target(selector) { ... }` 修改单个对象。
- `db update many Target { query } { ... }` 修改多个对象。
- `db upsert Target(key) { insertFields } { changes }` 按 key 保证存在，再应用 change。
- `db replace Target(selector) { ... }` 整对象覆盖。
- `db delete Target(selector)` 删除单个对象。
- `db delete many Target { query }` 删除多个对象。

change block 是持久更新 DSL，不是普通内存对象 mutation。它支持设置字段、数值增量 / 减量、清空 optional 字段、向集合字段添加值、从集合字段移除值。

限制：

- key 字段不能修改。
- 只能修改当前 object 的 stored field。
- 同一 change block 不能同时修改父路径和子路径。
- change block 不暴露 Mongo `$set`、`$inc`、`$push` 等 operator。

## 6. Result Types

DB read/write 返回类型不使用 `ReadRecord` 这类来源型 runtime descriptor。compiler 根据 DB metadata 直接生成普通类型：

- full object read：attached nominal type，例如 `User`。
- projected read：key 加 selected stored fields 的 anonymous record。
- `find` / `optional` 缺失可为空时，外层是 nullable。
- `find many` 外层是 `Array<...>`。
- `insert` / `update` / `replace` 单条返回 attached nominal type 或 `null`，按 operation 语义决定。
- `upsert` 返回 `{ value: <attached nominal type>, inserted: bool }`。
- `insert many` 返回 `{ insertedCount: number }`。
- `update many` 返回 `{ matchedCount: number, modifiedCount: number }`。
- `delete many` 返回 `{ deletedCount: number }`。
- `delete` / `exists` 返回 `bool`。
- `count` 返回 `number`。

`ReadRecord<User, fields:...>` 不属于 source-visible 类型，也不属于 runtime wire descriptor。若需要表达“User 的部分字段类型”，使用普通 anonymous record type：

```skiff
alias UserListItem = { id: string, name: string, visits: number }
```

## 7. Transaction

`db transaction` 是当前 service-owned database 内的原子 block。

```skiff
db transaction {
  const user = db require User(id)
  db update User(id) { visits += 1 }
}
```

`db transaction value` 是产值形态：

```skiff
const result = db transaction value {
  db require User(id)
}
```

语义：

- transaction 内 DB 读写在同一原子边界内执行。
- block 抛错时，未提交的 DB 写入回滚。
- 读取结果仍是 readonly snapshot。
- 所有持久写入必须显式使用 DB operation。
- 嵌套 transaction 当前不支持。
- transaction 冲突不自动重试。

transaction 内不应执行外部副作用或长时间工作，例如 HTTP、LLM、service call、actor call、`spawn` 或 `db claim`。actor routing、spawn 提交和外部副作用不随 DB rollback 回滚。

## 8. Lease

lease 是 `db object` 上的声明式单写者机制：在一段时间内，让至多一个执行者推进某个对象的工作。它保护的是“工作不被并发重复执行”；与条件更新表达的乐观并发（保护“写入不冲突”）互补，不互相替代。

lease 状态保存在对象所在文档内，与 service database 同生命周期：跨 runtime、跨 router 重启、跨 service version 有效。

### 8.1 声明

```skiff
type Thread {
  id: string
  currentRunId: string?
  inputSeq: number
}

db object Thread {
  primary key(id)
  lease drain ttl 60000 max 1800000
}
```

规则：

- `lease <name>` 在 db object 上声明一个具名租约槽；同一 db object 可声明多个互不影响的槽。
- `ttl` 是毫秒数，必填：持有者停止续租后，租约最迟这么久之后可被抢占。
- `max` 是毫秒数，可选：单次持有的硬上限。到达后 runtime 停止续租并取消持有者，用于收回卡死的持有者。
- 槽状态（owner、token、过期时间、request id）由平台管理，不属于 attached type 的 stored fields：不出现在 read snapshot、projection 和 change block 中。

### 8.2 Claim Block

`db claim` 是 try-claim：获取成功则执行块体并最终返回 `true`；槽被持有且未过期则不执行块体、立即返回 `false`。没有等待或排队语义。

```skiff
const claimed = db claim Thread(threadId).drain as thread {
  runDrainLoop(thread)
}
```

语义：

- `as <binding>` 绑定 claim 成功时读到的对象 full snapshot，可省略。
- 块体执行期间，runtime 自动续租，间隔小于 `ttl / 2`。
- 续租失败、槽被抢占或到达 `max` 时，runtime 取消块体执行，claim 以 lease-lost 平台错误结束。
- claim 成功时把当前 runtime request id 记入槽状态，供控制面诊断与后续的持有者取消能力使用。
- 块体正常结束或抛出业务错误时原子释放租约；进程级失败（crash、断连）不释放，由 `ttl` 过期回收。
- 过期租约可被新 claim 直接抢占，不需要专门的回收步骤。

约束：

- `db claim` 不允许出现在 `db transaction` 内：编译器拒绝词法可见的情形，动态进入（经函数调用）由 runtime 拒绝。块体内允许普通 transaction 和 `spawn`。
- 租约不可重入：当前 request 已持有某对象的某个槽时，对同一对象同一槽再次 claim 是平台错误。对其他对象实例的同名槽 claim 是普通 try-claim。
- 块体内 `spawn` 的调用在新 request 中执行，可能在本租约释放前就开始 try-claim 同一槽并得到 `false`。需要接力持有同一槽时，应在块体退出后再 `spawn`。

### 8.3 Fencing

守卫的作用域是动态的：claim 持有期间，当前 request 内对该 leased 对象实例（同一 db object、同一 primary key）的 `update` / `replace` / `delete`——无论写入语句位于哪个函数或 module——都自动追加租约守卫：提交时校验槽 token 仍属于当前持有者。守卫失败的写入不生效，并以 lease-lost 结束当前 claim。过期后复活的旧持有者由此被挡在写入之外。

`spawn` 提交的调用在新的 request 中执行，不继承持有关系：spawned call 对同一对象的写入不带守卫，也不受当前 claim 约束。跨 service 调用同理。

跨对象写入不被租约自动保护。需要与租约对齐的多对象写入，应放进包含至少一条 leased 对象写入的 `db transaction`：守卫失败使整个 transaction 回滚。

业务级终态拦截用普通条件更新表达。例如 stop：控制面先把状态推进到终态；持有者的后续写入携带 `where status == "running"` 之类业务条件，落空后自行退出。业务条件与租约守卫叠加生效。

### 8.4 槽状态读取

```skiff
const slot = db lease Thread(threadId).drain
```

返回 `{ owner: string, expiresAt: number, requestId: string? }?`；`null` 表示无人持有或已过期。用于诊断；平台提供按 request 取消的能力后，控制面用它定位并取消持有者。槽状态没有其它读写入口。

### 8.5 恢复

恢复不需要专门的过期扫描 surface：恢复方按业务状态找出“应该有持有者”的对象（例如 `currentRunId != null` 的 thread），逐个 try-claim。断租对象会被抢下并继续推进；仍被健康持有的对象 claim 返回 `false`，空跑退出。

### 8.6 当前不支持

- 阻塞 / 等待式 claim、公平排队。
- 租约重入、跨对象多行租约、租约转移。
- 按查询 claim 一个匹配对象（`db claim Target.slot { where ... }`）；方向已认可，未进入当前 surface。

## 9. Service-Owned Database

每个 service 在每个部署环境拥有自己的数据库命名空间。database identity 与稳定 service id 绑定，不包含 service version、build id 或 profile。

业务源码和 `service.yml` 不配置真实 DB 连接串。平台通过 router / runtime activation 下发 `serviceDb.mongoUrl`。业务代码不能读取连接串，也不能选择任意 database。

一个 service 默认不能直接读写另一个 service 的 database。跨 service 数据访问应通过 service API、事件复制或未来明确设计的只读投影视图。

底层 Mongo 映射是 adapter 细节。Skiff DB reference 不定义 Mongo collection API、Mongo filter、Mongo update operator 或索引创建流程。

## 10. Recoverable Stored Fields

DB stored field 是 owner-internal recoverable boundary。DB 的底线是“写入值必须可恢复”，再叠加 DB 自己的
projection、predicate 和 index policy。完整 recoverable contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

DB storage lane 分两类：

- schema-projectable lane：plain data、record、array、map 等不需要 code/carrier/adapter state 的字段保持现有 storage
  shape，可按本文件规则 projection、predicate、order 和 index。
- recoverable-envelope lane：静态类型图可能需要 code identity、`any I` carrier/self state、nominal behavior state、
  custom restore state 或 native adapter state 的 top-level stored field，整体存为 opaque recoverable envelope。

第一版 recoverable-envelope lane 不可穿透。示例：

```skiff
type RunBinding {
  id: string
  provider: any ToolProvider
}

db object RunBinding {
  primary key(id)
}
```

若 `provider` 是 `carrier = Local` 且 self payload 全可恢复，写入可成功；读出时按当前 expected type plan 和
recoverable compatibility contract 恢复为 `any ToolProvider`。若 `provider` 是 `carrier = Remote` 或 self 中含
stream / transaction / live connection / 无 adapter native handle，写入 fail closed，DB 不写半截 row。

对 `provider.someField` 做 `fields` projection、`where`、`order` 或 index 第一版不支持。需要可查询字段时，应把可查询事实
作为普通 schema-projectable 字段单独建模。

## 11. Current Unsupported Surface

当前不支持：

- 旧 `db.*` builtin surface。
- `db collection` 旧声明形态。
- collection 字符串、Mongo filter、Mongo update operator 作为业务 API。
- relation declaration、read-time relation target、`load` 和嵌套 load composition。
- 非 stored 字段，包括 relation、computed、memory、runtime field。
- 任意 unique query 作为 upsert selector。
- cursor / continuation / `after previousResult` / 隐藏 page provenance。
- 跨 service transaction。
- 自动 dirty tracking。
- 读取对象字段赋值后自动落库。
- 长批量修复作为普通在线 transaction。

## 12. Open Questions

未定问题只记录方向，不作为当前语义：

- cursor / page result 是否进入语言，以及签名、编码和跨版本兼容策略。
- array add / remove 对普通数组、set-like 数组和对象数组的精确定义。
- aggregation、全文搜索和 scan intent 是否进入语言核心。
- schema migration、字段 rename、backfill、索引 rollout、drift detection 和数据校验计划。
- DB conflict、constraint、not-found 等错误类型的正式 shape。
