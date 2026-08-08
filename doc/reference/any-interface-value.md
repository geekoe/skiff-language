# Skiff `any I` Runtime Value Boundary Reference

本文负责：从用户可见 reference 角度记录 `any I` 值跨边界时的判据。`any I` 语法、装箱和调用语义见
[`any-interface.md`](any-interface.md)；内部 value layout 见
[`../architecture/any-interface-value.md`](../architecture/any-interface-value.md)；可恢复值完整 contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

## Value And Execution Owner

record / `Array` / `Map` / `JsonObject` 及本地 `any I` payload 都是 value semantics。装箱、赋值、
普通参数传递、返回与 container store 产生逻辑 snapshot；Runtime 可以用 move/share/COW
共享 physical backing，但不得暴露 mutable alias。局部 `let` 与普通 parameter 不可写；局部
`var` 及从它派生的精确 writable path 可写；顶层 `const` 是 compiler-evaluated 且 deeply
frozen 的常量。显式 `inout` 只能作为 exact Package Local ABI、`NoPending` concrete call 的
exclusive loan；它不得出现在 interface requirement、callback、service/gateway/Actor external
contract、host effect 或任何 recoverable boundary。

每个正在执行的 request、stream producer 或 callback invocation 都由 exact deployment `buildId`
唯一固定到对应的 immutable execution image。这里的 execution owner 不是 deployment
activation generation，也不是全局 runtime assembly。Actor activation identity 或 transport socket
generation 可以作为各自子系统的寿命/传输事实，但不替代 deployment `buildId`。
owner-internal boundary 中的“owner”指 service trust domain；它与这个 runtime execution owner 是不同维度。

## Carrier

`any I` 的装箱点 `expr as I` 决定 runtime carrier：

- `carrier = Local`：装箱源是当前 exact deployment `buildId` execution image 中的 concrete nominal
  value。该值在同 request / 同 runtime 内可自由
  传参和放入内部集合；进入 DB、`dispatch`、queue / persistent work item 或 runtime 内部跨 request payload 时，必须按
  owner-internal recoverable boundary 编码。只有 self payload 全可恢复时才允许；否则 fail closed。
- `carrier = Remote`：装箱源是已发布 public instance，例如 `remoteLlm/managedLlm as LlmClient`。这是 request-scope
  正向远程引用，只用于持有方主动发起 consumer -> callee 调用。它不是 durable remote handle；进入任何
  recoverable boundary 时以 `recoverable_remote_carrier_not_persistable` fail closed。

Recoverable codec 不重新判断“本地还是远程”；它读取值中已经确定的 carrier。

## Boundary Rule

普通JSON/materialization、config schema和test double external fixture schema不允许`any I`默认wire shape，也不隐式
打开recoverable envelope。service operation有一个显式例外：顶层、非泛型`any I`参数、unary返回或server-stream
item可以投影为opaque callback capability，前提是`I`的全部method都可形成service boundary contract。capability由
创建该值的一侧拥有，并固定该侧 exact deployment `buildId` 及当前 runtime route；unary
调用中活到顶层request结束，server stream中延长到stream关闭。对端只能按`I`声明的operation
回调该 exact build owner，不能看到method table、本地地址或native object。runtime route 是传输坐标，
不是另一个 execution owner identity；socket generation 只能用于该 route 的连接寿命判定。

callback position是精确的类型位置规则，不按schema descriptor猜测：

- `any I`必须直接占据operation position，且`I`必须是非泛型、具有精确Package schema身份的interface；
- `Array<any I>`、record/nullable内嵌`any I`和泛型`any I<T>`第一版fail closed；
- 直接的Package schema仍按普通detached data处理，即使其descriptor是callback interface；
- raw function和没有显式callback adapter的native value继续不可用。
- callback interface 的 requirement 与投影后 operation 都不得含 `inout`；所有边界参数仍是
  detached logical snapshot。

callback capability在request或stream结束、owner退出或内部停止后失效，稳定返回`CapabilityExpired`或
`CapabilityUnavailable`；runtime不重建、不fallback，也不把它转换成recoverable envelope。

DB stored field、`dispatch` target 参数、queue / persistent work item payload 和 runtime 内部跨 request payload 是
owner-internal recoverable boundary。它们的底线是“值必须可恢复”，不是“`any I` 一律禁止”。`carrier = Local` 行为值走
`InterfaceValueState + self_node`；`InterfaceValueState` 不把 interface/projection 当 durable truth 保存，typed boundary 的
expected type plan 提供 interface/projection。`self_node` 携带 concrete value 的 stable `LocalConcrete` restore key 和可恢复
state，并按当前 execution context 恢复。若 expected type 是 union，多个 any-interface 分支都可匹配同一 local concrete 时
fail closed，不能按分支顺序猜测。`carrier = Remote` 或 self payload 不可恢复时，边界操作 fail closed，且不得写入半截
DB row、不得提交 dispatch/queue item。

`LocalConcrete` restore key 只表达 stable concrete identity：owner 是当前 service，或当前 linked program 中唯一的
package id；concrete identity 是该 concrete type 的稳定 ABI type identity。它不携带 service/package version、
build id、artifact identity、deployment owner、package slot、source path 或 runtime-local type address。恢复时只在当前
exact deployment `buildId` execution context 的 linked program 中查找这个 key；找不到 concrete、owner 不唯一、concrete 不再实现 expected
interface/projection，或 self state 不再符合当前 expected type 时，边界 fail closed。`any I` 不定义自动应用迁移，也不通过
runtime wrapper schema version 迁移旧 local self。
持久 key 不携带 buildId 与执行时必须 pin exact buildId 并不矛盾：前者避免把历史
implementation 身份变成 durable truth，后者决定本次 decode 可用的唯一 restore plan。

跨service只发送上述sealed opaque callback capability，绝不发送明文`LocalConcrete`、`NativeAdapter`、
`InterfaceValue` state、method table或本地地址。callback capability不能进入DB、`dispatch`、queue、persistent work
item或其它recoverable lane；离开owner service trust domain的显式recoverable slot第一版仍只允许plain data envelope。
`inout` loan 没有 recoverable encoding；任何尝试把 writable origin、loan token 或别名 path 带入这些
lane 都必须在边界提交前 fail closed。

## Examples

```skiff
let local: any ToolProvider = HostProvider { ... } as ToolProvider
dispatch drainWithProvider(local)      // allowed only if HostProvider self payload is recoverable

let remote: any ToolProvider = remoteLlm/remoteTools as ToolProvider
remote.listTools(ctx)              // request-scope forward remote call
dispatch drainWithProvider(remote)    // fail closed: Remote carrier is not persistable
```

Field rename、union branch rename、method projection mismatch 和其它跨版本变化不由 `any I` 自行迁移；recoverable decode
按 [`static-semantics.md §18.1`](static-semantics.md#181-recoverable-compatibility-contract) 的精确身份矩阵判定。
