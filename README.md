# Skiff（轻舟）

Skiff，AI时代的后端编程语言。

中文名“轻舟”，寓意：AI写后端，已过万重山。

Skiff 让 AI 和工程师用同一种语言描述后端服务：API 怎么开放，数据怎么保存，后台任务怎么推进，
流式响应怎么结束，测试和发布要守住哪些边界。它不是把一组框架约定交给人记住，而是把这些后端
规则写进语言、编译器和运行时里。

Skiff 仍处于发布前阶段。当前仓库不为旧语法、旧 artifact 或旧配置格式提供兼容层；实现和文档
都以 `doc/reference/` 中的目标语义为准。部分能力在 reference 中是目标态设计，落地状态以当前
实现和测试为准。

## 语言定位

后端开发里很多关键问题过去依赖框架经验和团队约定：接口是否稳定、数据是否可恢复、后台任务会
不会重复执行、流式调用如何取消、测试替身是否覆盖真实外部调用。Skiff 的目标是把这些问题变成
语言可以检查的事实。

- AI 可以直接生成服务代码、数据模型、测试和后台流程，而不是拼接一堆隐含规则的 glue code。
- API、数据库对象、后台任务、配置、日志和测试替身都在同一套模型里描述。
- 跨服务调用、持久化数据和异步执行都有明确边界，能在编译、测试、发布和运行时被检查。
- 当系统无法证明某个边界是安全的，它应该明确失败，而不是带着不兼容或不可恢复的状态继续运行。

## 类型系统

Skiff 以名义类型为主。`type Name = R` 创建新的 representation 身份，不是透明别名；已有 payload
不会被目标类型上下文隐式包成另一个名义值。需要缩写时使用 `alias Name = R`，它在语义上按 RHS
展开。

命名 union 也有自己的身份。具体名义分支、带 discriminator 的匿名 record 分支和 literal 分支都会
参与可恢复的 runtime identity。两个 union 即使分支形状相同，也不会结构化互换。

基础 surface 包含 `string`、`number`、`integer`、`bool`、`null`、`Date`、`bytes`、`Json`、
`JsonObject`、`Array<T>`、`Map<K,V>`、`Stream<T>`、`void` 和 `never`。`integer` 是 safe integer
refinement，运行时仍由 `number` 表示；`Json` / `JsonObject` 是裸 JSON 数据，不保留用户名义身份。

record、array、map 和 object 是可变引用语义。`const` 只限制 binding 不能重新绑定，不表示 deep
immutable。进入 API、DB、spawn、queue 或其它跨 request 边界的类型必须满足对应的 schema closure
或 recoverable value policy。

## Interface 与能力值

`interface` 是名义能力契约，不是结构类型，也不是数据 shape。conformance 必须由 nominal record
显式写 `implements`，编译器不会因为字段或 method set 相似就自动匹配。

裸 interface 名不能作为普通值类型传递、存储或进入边界。需要可流动的动态能力时，类型写成
`any I`，装箱点显式写 `expr as I`：

```skiff
interface ToolProvider {
  function listTools(self: Self) -> Array<ToolDefinition>
}

type LocalTools implements ToolProvider {
  name: string
}

function chooseProvider() -> any ToolProvider {
  return LocalTools { name: "local" } as ToolProvider
}
```

`any I` 是 opt-in 的动态分派。本地装箱值携带 concrete payload 和 method table；远程 public
instance 装箱值携带 dependency / public instance / operation 坐标，例如：

```skiff
function chooseLlm() -> any LlmClient {
  return remoteLlm/managedLlm as LlmClient
}
```

package 抽象能力也通过 `any I` 参数表达，由调用方在调用点选择本地实现或远程 public instance。
旧式 capability binding 已退役。

## Source、Import 与 Public API

Skiff source file 是代码组织单位，不是包内 privacy 边界。当前 publication 内跨文件访问使用
内建 `root.<module>.<Symbol>`；外部 package 通过 manifest/package 配置中的 alias 引入，并在源码中
写 `import llm` 这类 simple local package name。

public API 不写在 `.skiff` 源码的 `export` 关键字里，而由 publication source root 下的 `api.yml`
显式声明。`api.yml` 左侧 public path 是外部源码可写名字，右侧 source selector 绑定当前 production
source set 中的顶层声明。public root 引用到的 named type 会自动进入 ABI / schema closure，但不会
自动变成外部可写 public name。

package 和 service 共享同一套 Publication API graph。package 是本地链接形态；service 是远程运行形态，
会把 public callable 和 public instance method projection 成以 `operation_abi_id` 为核心的 runtime
operation。service protocol identity 来自 public operation、canonical signature、schema closure、
public instance metadata 和 service dependency lock，而不是源码文件路径、HTTP path 或 display name。

## 运行时模型

Skiff runtime 执行 typed operation dispatch。Gateway 负责外部协议适配，router 负责 service runtime
注册、路由、drain 和 control plane，service runtime 才执行用户 Skiff 代码。

每次 unary call、server-stream call、HTTP entry、WebSocket connect/receive 或测试 dispatch 都创建
独立 request frame。request frame 包含参数、deadline、trace、call frame、request-local heap、
exception envelope、stream sink/source handle、concurrent lane state 和资源句柄；请求结束后这些
状态整体清理。

`concurrent` 是结构化并发语义，不是后台任务。它把直属 statement 或 `serial { ... }` 划成 lane，
通过 effect metadata、mutable root provenance 和 source order 建立可执行 DAG。外层 mutable root
写入、stream `emit`、直接 `throw` / `catch`、`spawn` 和普通复杂控制流都不能放在 concurrent surface
里。

`timeout(...)` 只收紧当前 block 和其中远程 / host 调用的有效 deadline。cancel 是明确 runtime 事件：
断线、caller cancel、timeout、consumer 提前停止 stream 迭代和 drain 都必须收敛为 cancel 或 error
语义，而不是静默丢掉 in-flight request。

`Stream<T>` 当前只支持 server / source stream。返回 `Stream<T>` 的 service operation 用 `emit expr`
输出有序 chunk；stream 是一次性 request-local 值，不能作为普通用户 operation 参数、record 字段、
持久化字段或 collection 元素。

## Service-Owned Database

Skiff DB 是 service-owned typed object database。业务代码面对的是 `db object`、typed query、
projection 和显式 write operation，而不是 collection 字符串、Mongo filter 或 update operator。

```skiff
type User {
  id: string
  name: string
  visits: number
}

db object User {
  name "user"
  primary key(id)
}

const users = db find many User {
  fields { name, visits }
  where visits > 0
  order id asc
  limit 20
}
```

读取结果是 readonly snapshot / projection；字段赋值不会自动写回数据库。持久修改必须使用
`db insert`、`db update`、`db upsert`、`db replace`、`db delete` 等显式 operation。

`db transaction` 提供当前 service database 内的原子 block。`db claim` 提供声明式 lease，用于让至多
一个执行者推进某个对象的工作，并用 fencing 防止旧持有者继续写入。

DB stored field 是 owner-internal recoverable boundary。普通可投影数据走 schema-projectable lane；
需要代码身份、`any I` carrier/self state 或 native adapter state 的字段走 recoverable-envelope lane，
第一版不能对其内部字段做 projection、predicate、order 或 index。

## 后台任务、队列和文件

`spawn` 是语言层唯一的后台调用 surface。它提交一次当前 service 或 linked package 中的普通函数调用，
当前 request 继续执行。spawn target 不能是跨 service callable，返回类型必须是 `void` / `null`，
参数必须是可恢复值。提交成功后，spawned call 在新的 request frame 中执行，不继承 caller 的
request-local 状态，也不自动重试。

durable queue / timer 是平台调度机制，用于同步请求、后台任务、实体事件推进和未来唤醒。queue payload
同样是 recoverable boundary；平台第一版不提供 exactly-once，也不自动重跑失败 item。长时业务的关键
事实必须落在 service-owned DB 或业务 durable state 中。

文件能力以不可变 `ROFile` 为默认语义，通过 `FileRef` 引用共享后端中的完整文件。追加日志用分段
AppendLog 建模；需要真实路径的工具链应使用显式 workspace / volume 能力。Skiff 不把本地 runtime
磁盘或普通 POSIX 文件系统作为生产 source of truth，也不在普通 request handler 中直接暴露任意
shell command API。

## Prelude 与 `std`

prelude 类型和基础 receiver API 默认可见。`std` 是内建平台标准库 root，不是普通 package dependency。
源码可以直接访问 `std.http.*`、`std.json.*`、`std.crypto.*`、`std.time.*`、`std.log.*` 和
`std.websocket.*`；`config` 是独立内建 value root，用来读取当前 request frame 注入的 typed config view。

`std` API 必须声明 effect metadata，包括 target、conflict-key、cancel safety、stream / callback
行为和测试替身边界。HTTP、WebSocket、service call、telemetry、crypto/random、time sleep 等 host-backed
能力都通过 runtime 管控；业务代码不能绕过这些边界直接访问宿主对象。

`std.log` 只产生 best-effort telemetry 事件。telemetry 用于诊断、trace、metric 和 health，不承载审计、
扣费、outbox 或任何丢失后会改变业务正确性的事件。

## 测试模型

Skiff 只保留一种测试源码语义：`test` block。它只能出现在 `*.test.skiff` 文件中，`assert` 只能出现在
`test` block 中。test-only source 不进入 production artifact、package assembly、service assembly、
public API surface 或 config metadata。

同目录 `foo.test.skiff` 和 `foo.*.test.skiff` 可以作为 `foo.skiff` 的 friend test，获得对应生产文件的
白盒访问权。runner mode 决定测试宿主和 effect policy：VM / unit mode 不访问真实网络，外部 effect
必须由 test double 替换；runtime / integration mode 使用真实 Skiff runtime 语义；live smoke 应显式
运行并使用 `test defaultRun false` 避免默认执行。

测试替身按 stable target id 匹配，必须返回 schema-closed payload 或抛标准 `ErrorPayload` leaf。mock
仍参与 effect summary，不能绕过 `concurrent` 冲突检查。

## 阅读路线

- 语言语法：`doc/reference/syntax.md`
- 类型和边界：`doc/reference/static-semantics.md`
- interface / 动态能力：`doc/reference/interface.md`、`doc/reference/any-interface.md`
- runtime：`doc/reference/runtime.md`
- 标准库 surface：`doc/reference/std-surface.md`
- DB / spawn / queue：`doc/reference/db.md`、`doc/reference/spawn.md`、`doc/reference/queue.md`
- 发布和 ABI：`doc/reference/publication-api-yml.md`、`doc/reference/publication.md`
- 测试：`doc/reference/testing.md`

## 仓库入口

- `compiler/`、`syntax/`、`artifact-model/`、`artifact-identity/`：compiler 和 artifact identity。
- `runtime/`：Rust runtime crates 和 host process。
- `router/`：TypeScript service router 和 runtime control plane。
- `telemetry/`：TypeScript telemetry process。
- `scripts/`：CLI 和本地 instance 工具。
- `std/`、`prelude/`：Skiff 标准库源码。
- `test-runner/`：Skiff package 和 service 测试基础设施。
- `doc/`：canonical language、runtime 和 architecture 文档。

开发协作说明见 `AGENTS.md`。许可证见 `LICENSE`。
