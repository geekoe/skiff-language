# Skiff DB Reference

本文负责：稳定描述 Skiff service-owned database 的用户可见语言规则，包括 `db object`、读写操作、query block、projection、返回类型、transaction、lease、数据库归属、encrypted storage mapping 和当前不支持事项。

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
- `name`声明Package内部稳定的logical collection identity，不是Mongo physical collection name。
- 省略`name`时使用该`db object`的canonical object identity作为logical collection identity。
- Runtime按`(packageId, declared logical collection identity)`确定性编码physical collection name；
  dependency、deployment或配置中不存在作者提供的physical collection mapping。
- `db object` 不生成 `Row`、`Document`、`Entity` 等额外源码名字。

### 1.1 Test Dependency DB Targets

`kind: test` service可以通过direct package dependency的`topLevelAlias`使用其文件顶层`db object`。
DB target沿用测试顶层名字语法：

```skiff
const session = db require subjectImpl/model.AdminSession(sessionId)
```

这里的`subjectImpl/model.AdminSession`同时选择provider package中的`model.AdminSession` type及同文件、
同名的`db object AdminSession` attachment。它不会把该attachment公开到`api.yml`，也不允许普通
alias、transitive dependency或production service获得内部DB访问。旧`access: topLevel`字段不再合法。

该规则覆盖所有带target的DB语法：key/query read、insert/update/upsert/replace/delete、count/exists、
`DbQuery`值、lease claim、lease状态读取，以及claim期间自动追加的写入guard。`db transaction`是当前
service database上的词法执行边界，本身没有target；其中每个DB operation仍独立按上述规则解析。

两个dependency即使声明相同module path与type name也不会混淆：dependency alias选择精确package
artifact。运行时仍写入当前test service拥有的唯一database；跨package target不是跨service
database访问。该数据库由平台按`(testRunId, generatedTestServiceId)`派生，不由测试配置命名。两个
dependency的logical collection分别由stable
`(packageId, declared logical collection identity)`系统编码；作者不提供physical mapping。

同一含DB metadata的Package可因direct与transitive依赖形成菱形。若两条edge解析到同一精确build且
owner-relevant facts相同，activation将其合并为一个metadata owner；同一Package ID解析到不同build、
logical collection identity缺失/重复或system physical-name encoding collision都必须失败。

普通service同样只有一个数据库。数据库identity由operator选择的受信Mongo endpoint/storage domain、
activation environment与serviceId共同定界；不另设`platformId`。开发者不能通过`package.yml`或配置文件
选择database/namespace；同一service中的所有Package共享该数据库，同时每个DB target继续使用精确
PackageArtifact/File IR/type identity。不同Package可以使用相同裸collection名字而不共享storage；
跨service数据库访问禁止。

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
使用 encrypted storage mapping 的字段同样只能作为整个 top-level 字段读写，不能作为 field path 的父级，也不能
用于 predicate、order 或 index。

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
- block抛出普通业务错误或在commit选择前正常失败时，runtime等待abort，未提交的DB写入回滚。
- deadline、ancestor stop、disconnect或execution future drop属于异常内部停止：runtime尽力abort，
  但可见request结果不等待abort acknowledgement；driver/session关闭是最终fallback。
- block正常完成后runtime选择commit。Commit尝试一旦开始，后续内部停止不改选“保证abort”；
  底层commit可能完成或处于unknown outcome，late result被丢弃。内部停止、断线或timeout都不承诺
  撤销已经开始的commit。
- 读取结果仍是 readonly snapshot。
- 所有持久写入必须显式使用 DB operation。
- 嵌套 transaction 当前不支持。
- transaction 冲突不自动重试。数据库写冲突或 transient transaction conflict 会归一为可捕获的
  `std.db.ConflictError { target: "std.db", message: string, retryable: true }`；错误消息是稳定、
  脱敏的 DB 冲突说明，不包含 Mongo 原始详情。调用方只应在确认整个重试边界不包含外部副作用时
  显式重试。

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
- `max` 是毫秒数，可选：单次持有的硬上限。到达后runtime停止续租并内部停止块体，以
  lease-lost结束，用于收回卡死的持有者。
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
- 续租失败、槽被抢占或到达`max`时，runtime内部停止块体执行，claim以lease-lost平台错误结束。
- claim成功时把当前runtime request id记入槽状态，只供诊断和trace关联。
- 块体正常结束或抛出业务错误时runtime停止续租并等待release；release完成后才结束正常路径。
- deadline、ancestor stop、disconnect或execution future drop时，runtime先停止续租并尽力release，
  但不要求可见request结果等待release acknowledgement；未能及时release时由`ttl`过期回收。
- 进程级失败（crash、断连）同样不承诺主动release，由`ttl`过期回收。
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

返回 `{ owner: string, expiresAt: number, requestId: string? }?`；`null` 表示无人持有或已过期。
这些字段只用于诊断、trace关联和恢复观察，不提供按request取消持有者的控制面能力。槽状态没有其它
读写入口。

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

## 10. Encrypted Stored Fields

Encrypted stored field 用于把低频 secret string（例如 API key）以认证密文形式存入 service-owned database，同时在
Skiff 源码、service API 和运行时内存中维持普通 `string` 语义。它不是新的 secret 类型，也不会阻止业务代码记录、
返回或以其他方式泄漏已经解密的值。

### 10.1 声明与静态约束

Canonical syntax 是在 `db object` 中为已有 stored field 增加 storage mapping：

```skiff
type ProviderCredential {
  id: string
  provider: string
  apiKey: string
}

db object ProviderCredential {
  name "provider_credential"
  primary key(id)
  storage apiKey using encrypted
}
```

约束：

- `storage`、`using`、`encrypted` 只在 `db object` declaration 中是 contextual keyword；`storage` 后只接受单个
  top-level identifier，`using` 后只接受 `encrypted`，行尾分号沿用其他 DB entry 的可选规则。
- encrypted field 必须是 top-level、非 nullable 的精确 `string`；最终展开为 `string` 的 alias 也可以。
- 声明 encrypted field 的 object，其单字段 primary key 也必须是非 nullable `string`。
- primary key 自身不能 encrypted；encrypted field 也不能 nullable、nested、indexed，不能与 recoverable-envelope 或
  immutable-file storage mapping 重叠。
- 同一字段重复声明 storage mapping 是 compile error；未声明 mapping 的字段保持 identity storage behavior。
- `storage ... using encrypted` 只声明物理存储方式，不改变源码 field type、构造规则、projection 返回类型或 service
  protocol ABI。调用方不能选择算法、nonce、key id 或密文格式。
- encrypted field 面向 API key 等低频 secret。单个 derived field key 达到 `2^32` 次写入前必须轮换 root key；
  runtime 不维护持久写次数计数，容量核算属于运维责任。

### 10.2 Operation contract

| Operation / field use | Contract |
| --- | --- |
| full read，或 `fields { apiKey }` | 允许；runtime 在返回前解密，projection 仍自动包含 primary key |
| `where`、`order`、index，或 nested path | 拒绝；不提供 searchable encryption |
| `db insert` / `db insert many` | 允许；每行、每个 encrypted field 独立生成随机 nonce |
| `db replace` | 允许；replacement 必须提供可确定的 string record id |
| 按 key 的 `db update` / `db upsert`，top-level `set` encrypted field | 允许；selector key 进入认证上下文 |
| query selector / `update many` 设置 encrypted field | 拒绝；多行写入不能复用 ciphertext 或缺失 record id |
| `unset`、`add`、`remove`、数值 change 或 nested change | 拒绝 |

query、count 和 exists 可以继续使用同一 object 的普通可查询字段，但任何 predicate 都不能引用 encrypted field。
读取完整 row 会解密其中全部 encrypted field；只需要 secret 时可显式使用 `fields { apiKey }`，其逻辑结果仍包含
primary key，例如 `{ id: string, apiKey: string }`。

### 10.3 密文、认证上下文与失败语义

当前格式固定使用 32-byte root key、HKDF-SHA256 派生的 32-byte field key、AES-256-GCM、每次写入由 OS CSPRNG
生成的 12-byte nonce，以及原始 UTF-8 string bytes 作为 plaintext。每个 root key id、storage service、最终物理
collection 和 top-level field 都派生出不同的 field key。

物理 BSON 值是 Skiff 保留 envelope：

```javascript
{
  _skiff_encrypted: {
    version: 1,
    keyId: "2026-01",
    nonce: BinData(0, "..."),
    ciphertext: BinData(0, "...")
  }
}
```

AEAD additional authenticated data（AAD）确定性绑定以下逻辑上下文：

```text
keyId
storageServiceId
finalPhysicalCollectionName
topLevelFieldName
logicalStringRecordId
```

因此同一明文重复写入会得到不同密文；把合法 envelope 移到另一个 record、service、collection 或 field，修改
`keyId` / nonce / ciphertext，或使用错误 key 都会认证失败。缺字段、多字段、错误 BSON type、未知 version、未知
key id、认证失败和 UTF-8 失败都返回 sanitized DB decode error；runtime 不接受明文 string fallback，也不会把
malformed envelope 当成普通业务 record。当前格式没有数据库之外的可信 freshness state，所以同一
record/service/collection/field 上的历史合法 envelope 回放不在防护范围内。

该能力保护 Mongo 数据文件、备份和磁盘副本泄漏，以及跨上下文复制或篡改；它不保护已经控制 runtime 进程、能读取
keyring、业务代码主动暴露 plaintext 或进程 memory dump 的攻击者。解密后的 `string` 会进入 runtime / service 内存；
应用仍必须避免把它写入日志、错误、telemetry 或 API response。

### 10.4 Runtime host keyring

Keyring 是 runtime host secret，不属于 router、service activation、control frame、resolved service config 或 artifact。
`runtime.yml` 中的唯一引用是：

```yaml
serviceDb:
  encryption:
    keyringFile: /run/secrets/skiff-service-db-keyring.json
```

相对路径按 `runtime.yml` 所在目录解析。生产部署应使用远端 runtime host 上的绝对 secret-mount 路径，并在运行
deploy script 前由部署平台 provision 文件；deploy script 只把路径写入 `runtime.yml`，不读取、传输、创建或备份
keyring，也不在部署摘要中输出路径或 material。

Keyring 文件格式：

```json
{
  "format": "skiff-service-db-keyring-v1",
  "activeKeyId": "2026-01",
  "keys": {
    "2026-01": "<canonical-base64-32-bytes>",
    "2025-02": "<canonical-base64-32-bytes>"
  }
}
```

`activeKeyId` 指定新写入使用的 key，`keys` 中保留的旧 key 只用于读取历史 envelope。每个 key 是 32-byte 随机
material 的 RFC 4648 canonical Base64 表示；key id 最多 64 UTF-8 bytes，只允许 `[A-Za-z0-9._-]`。Unix 上必须是
regular file，且 group/other permission bits 为 `0`（推荐 `0400` 或 `0600`）。空文件、缺失、权限不安全、重复 key、
未知字段、无效格式或 active key 缺失都会让 runtime 启动失败；文件只在 runtime 启动时读取，更新后必须 restart。

runtime 未配置 keyring 时，不含 encrypted field 的 service 正常激活；含 encrypted field 的 service 在 DB provider
build 阶段 fail closed，不注册可用路由。runtime 不生成临时 key，也不把字段降级为明文。

同一个 storage service 的所有 runtime replica 必须挂载完全相同的 keyring。runtime 成功加载后发出不含路径或
material 的 `service_db.encryption_keyring_loaded` 事件，携带 format、active key id 和覆盖整个 keyring 的非 secret
fingerprint；部署与轮换必须用该事件核对 replica 一致性。

### 10.5 启用、发布、重命名与灾难恢复

Encrypted mapping 只允许在全新 DB object / 物理 collection，或已确认完全为空的 collection 上启用。已有非空
collection 的明文 string 不会自动升级：直接增加 mapping 会让旧 string fail closed；新增非 nullable encrypted
field 也会让缺字段的旧 row 失败。需要保留数据时，必须新建使用最终 storage identity 的 object / collection，通过
正常 encrypted insert 做 out-of-place copy，并在停写窗口校验和切流。

发布顺序固定为：

1. 部署理解 encrypted metadata / codec、但尚未加载 encrypted schema 的 runtime。
2. 在所有 runtime replica 上安装相同 keyring，并用加载事件核对 fingerprint。
3. 加载含 encrypted field 的 service artifact。
4. 最后开放业务写入。

一旦写入 envelope，旧 runtime / artifact 无法把保留 BSON document 当成 plaintext string。回滚 service artifact 前必须
停写并迁移或清空 encrypted 数据，不能只删除 `storage` declaration。AAD 使用最终
`storageServiceId + physicalCollectionName + fieldName`；更改 service storage identity、Package ID、
logical collection identity、system physical-name encoding或field name
都会让旧密文无法解密，必须按显式存储迁移处理，不能把 rename 当作 metadata-only change。

Keyring 丢失等同于密文不可恢复。数据库备份与 keyring 备份必须分开保存、分别授权，并进行成对恢复演练；恢复任一可能
包含旧 envelope 的数据库备份时，recovery keyring 必须仍含对应旧 key。不要把 key material 写入源码、artifact、deploy
manifest、日志或数据库备份。

### 10.6 停写轮换 runbook

轮换作用域是整个 **keyring deployment cohort**：所有加载该 keyring material、或曾用其中 key 写入密文的 runtime、
writer 和 `storageServiceId`。当前只支持维护窗口内的停写轮换，不提供在线轮换或通用 rotation CLI。

1. 从部署 inventory 枚举整个 cohort，包括所有当前/历史 writer、`storageServiceId`，以及最终物理 collection/field；
   inventory 不完整时停止，继续保留旧 key。
2. 阻断 cohort 内全部 service 的新业务写入，drain 正在执行的请求，并停止所有 runtime replica / writer。
3. 在每个 runtime 安装完全相同的 old+new keyring，把 `activeKeyId` 切到新 key。
4. 启动所有 replica，但维持写屏障；确认每个进程的加载事件都有相同 `format + activeKeyId + keyringFingerprint`，且没有
   旧 writer 存活。
5. 对 inventory 中每个 `storageServiceId + finalPhysicalCollectionName` 运行一次服务迁移。按 string primary key 做
   `where id > lastId order id asc limit batchSize` 分页，读出一行全部 encrypted field 的 plaintext，再在同一次按-key
   update 中 top-level set 回当前值。checkpoint key 固定为
   `(targetKeyringFingerprint, storageServiceId, finalPhysicalCollectionName)`，每批 transaction 完成后记录 `lastId`；
   写入后、checkpoint 前崩溃时可安全重跑该批。
6. 具备受控、只读 Mongo 运维权限的操作者对 inventory 中每个 encrypted field 验证 active key，且在全部扫描归零前
   保持写屏障：

   ```javascript
   db.getCollection("<collection>").countDocuments({
     "<field>._skiff_encrypted.keyId": { $ne: "NEW_ID" }
   })
   ```

   `$ne` 也匹配缺字段；服务迁移的全量 read 会另外让 malformed envelope fail closed。保存 inventory、扫描命令和结果
   作为轮换记录。
7. 再次停止 cohort 全部 replica / writer，从在线 keyring 删除旧 key；启动后用新的共同 fingerprint 再次确认一致。
8. 解除 cohort 写屏障。旧 material 进入与数据库备份分离的离线 recovery keyring，直到所有可能包含旧 envelope 的备份
   过期或已完成恢复后重加密，不能立即销毁。

普通 read 不隐式写回旧 key envelope。不能在并发写入时用 read+set 轮换，不能只迁移一个 storage service、只滚动部分
replica，或在 cohort 外 writer 尚未纳管时删除旧 key。

### 10.7 当前非目标

Encrypted stored fields 当前不提供 KMS / HSM 接入、自动 key rotation、在线 re-encryption、searchable / deterministic
encryption、nullable 或 nested encrypted field、用户自定义 codec、通用 schema migration、plaintext 自动升级、secret
taint type，或对业务内存和输出的自动脱敏。

## 11. Recoverable Stored Fields

DB stored field 是 owner-internal recoverable boundary。DB 的底线是“写入值必须可恢复”，再叠加 DB 自己的
projection、predicate 和 index policy。完整 recoverable contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

DB recoverability lane 分两类：

- schema-projectable lane：plain data、record、array、map 等不需要 code/carrier/adapter state 的字段保持现有 storage
  shape，可按本文件规则 projection、predicate、order 和 index。
- recoverable-envelope lane：静态类型图可能需要 code identity、`any I` carrier/self state、nominal behavior state、
  custom restore state 或 native adapter state 的 top-level stored field，整体存为 opaque recoverable envelope。

Encrypted mapping 是 schema-projectable `string` 的额外物理 storage policy，不是第三种 runtime value lane；它的查询和
写入限制见上一节。

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

## 12. Current Unsupported Surface

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

## 13. Open Questions

未定问题只记录方向，不作为当前语义：

- cursor / page result 是否进入语言，以及签名、编码和跨版本兼容策略。
- array add / remove 对普通数组、set-like 数组和对象数组的精确定义。
- aggregation、全文搜索和 scan intent 是否进入语言核心。
- schema migration、字段 rename、backfill、索引 rollout、drift detection 和数据校验计划。
- DB conflict、constraint、not-found 等错误类型的正式 shape。
