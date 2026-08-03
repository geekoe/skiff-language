# Skiff `any I` Runtime Value Boundary Reference

本文负责：从用户可见 reference 角度记录 `any I` 值跨边界时的判据。`any I` 语法、装箱和调用语义见
[`any-interface.md`](any-interface.md)；内部 value layout 见
[`../architecture/any-interface-value.md`](../architecture/any-interface-value.md)；可恢复值完整 contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

## Carrier

`any I` 的装箱点 `expr as I` 决定 runtime carrier：

- `carrier = Local`：装箱源是当前 service/runtime 中的 concrete nominal value。该值在同 request / 同 runtime 内可自由
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
创建该值的一侧拥有；unary调用中活到顶层request结束，server stream中延长到stream关闭。对端只能按`I`声明的
operation回调owner，不能看到method table、本地地址或native object。

callback position是精确的类型位置规则，不按schema descriptor猜测：

- `any I`必须直接占据operation position，且`I`必须是非泛型、具有精确Package schema身份的interface；
- `Array<any I>`、record/nullable内嵌`any I`和泛型`any I<T>`第一版fail closed；
- 直接的Package schema仍按普通detached data处理，即使其descriptor是callback interface；
- raw function和没有显式callback adapter的native value继续不可用。

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
build id、artifact identity、activation identity、package slot、source path 或 runtime-local type address。恢复时只在当前
execution context 的 linked program 中查找这个 key；找不到 concrete、owner 不唯一、concrete 不再实现 expected
interface/projection，或 self state 不再符合当前 expected type 时，边界 fail closed。`any I` 不定义自动应用迁移，也不通过
runtime wrapper schema version 迁移旧 local self。

跨service只发送上述sealed opaque callback capability，绝不发送明文`LocalConcrete`、`NativeAdapter`、
`InterfaceValue` state、method table或本地地址。callback capability不能进入DB、`dispatch`、queue、persistent work
item或其它recoverable lane；离开owner service trust domain的显式recoverable slot第一版仍只允许plain data envelope。

## Examples

```skiff
const local: any ToolProvider = HostProvider { ... } as ToolProvider
dispatch drainWithProvider(local)      // allowed only if HostProvider self payload is recoverable

const remote: any ToolProvider = remoteLlm/remoteTools as ToolProvider
remote.listTools(ctx)              // request-scope forward remote call
dispatch drainWithProvider(remote)    // fail closed: Remote carrier is not persistable
```

Field rename、union branch rename、method projection mismatch 和其它跨版本变化不由 `any I` 自行迁移；recoverable decode
按 [`static-semantics.md §18.1`](static-semantics.md#181-recoverable-compatibility-contract) 的精确身份矩阵判定。
