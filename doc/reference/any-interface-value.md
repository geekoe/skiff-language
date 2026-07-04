# Skiff `any I` Runtime Value Boundary Reference

本文负责：从用户可见 reference 角度记录 `any I` 值跨边界时的判据。`any I` 语法、装箱和调用语义见
[`any-interface.md`](any-interface.md)；内部 value layout 见
[`../architecture/any-interface-value.md`](../architecture/any-interface-value.md)；可恢复值完整 contract 见
[`../architecture/recoverable-value.md`](../architecture/recoverable-value.md)。

## Carrier

`any I` 的装箱点 `expr as I` 决定 runtime carrier：

- `carrier = Local`：装箱源是当前 service/runtime 中的 concrete nominal value。该值在同 request / 同 runtime 内可自由
  传参和放入内部集合；进入 DB、`spawn`、queue / persistent work item 或 runtime 内部跨 request payload 时，必须按
  owner-internal recoverable boundary 编码。只有 self payload 全可恢复时才允许；否则 fail closed。
- `carrier = Remote`：装箱源是已发布 public instance，例如 `remoteLlm/managedLlm as LlmClient`。这是 request-scope
  正向远程引用，只用于持有方主动发起 consumer -> callee 调用。它不是 durable remote handle；进入任何
  recoverable boundary 时以 `recoverable_remote_carrier_not_persistable` fail closed。

Recoverable codec 不重新判断“本地还是远程”；它读取值中已经确定的 carrier。

## Boundary Rule

普通 service/public API schema、ordinary JSON/materialization、config schema 和 test double external fixture schema 不允许
`any I` 默认 wire shape，也不隐式打开 recoverable envelope。

DB stored field、`spawn` target 参数、queue / persistent work item payload 和 runtime 内部跨 request payload 是
owner-internal recoverable boundary。它们的底线是“值必须可恢复”，不是“`any I` 一律禁止”。`carrier = Local` 行为值走
`InterfaceValueState + self_node`；`InterfaceValueState` 不把 interface/projection 当 durable truth 保存，typed boundary 的
expected type plan 提供 interface/projection。`self_node` 携带 concrete value 的 stable `LocalConcrete` restore key 和可恢复
state，并按当前 execution context 恢复。若 expected type 是 union，多个 any-interface 分支都可匹配同一 local concrete 时
fail closed，不能按分支顺序猜测。`carrier = Remote` 或 self payload 不可恢复时，边界操作 fail closed，且不得写入半截
DB row、不得提交 spawn/queue item。

跨 service 行为值第一版 fail closed。目标态需要 sealed opaque payload 与 service callback transport；这些能力落地前，
不得把明文 `LocalConcrete` / `NativeAdapter` / `InterfaceValue` state 发给对端 service 或 public client。离开 owner service
trust domain 的显式 recoverable slot 第一版只允许 plain data envelope。

## Examples

```skiff
const local: any ToolProvider = HostProvider { ... } as ToolProvider
spawn drainWithProvider(local)      // allowed only if HostProvider self payload is recoverable

const remote: any ToolProvider = remoteLlm/remoteTools as ToolProvider
remote.listTools(ctx)              // request-scope forward remote call
spawn drainWithProvider(remote)    // fail closed: Remote carrier is not persistable
```

Field rename、union branch rename、method projection mismatch 和其它跨版本变化不由 `any I` 自行迁移；recoverable decode
按 [`static-semantics.md §18.1`](static-semantics.md#181-recoverable-compatibility-contract) 的精确身份矩阵判定。
