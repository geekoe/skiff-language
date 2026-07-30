# P5-F305 Platform catch identity consumer audit结果

状态：Completed read-only。

## 结论

canonical `WirePayload::catch_projection`已经返回`CatchIdentity`。旧consumer不得再构造任意字符串
`TypeIdentity::builtin`，只能使用F299 model中冻结的有限
`PlatformBuiltinErrorIdentity::<Variant>.catch_identity()`。

合法映射覆盖Cancel、Timeout、Config/Bytes/Number/Json/Db/Time decode、DbConflict、File、
ServiceProviderUnavailable、ServiceProtocol与Http。动态decode code必须使用
`PlatformBuiltinErrorIdentity::from_symbol`并fail closed。

唯一行为修正：`std.resource.ResourceError`是Package-owned public type，不在platform registry中。
native/eval旧projection必须改为`None`；其未来catchability只能来自actual carrier与
`ServiceErrorTypeIndex`，不能按payload code猜identity。

`std.service.InternalError`同样不是platform string identity；它已有fixed service envelope与ordinary
nominal owner，本迁移不得加入registry。

## 最短DAG

```text
F299 model/boundary checkpoint
├── R1 capability-context
│   ├── R3 native
│   └── R4 service-db
└── R2 linked-type-plan

(R1, R2, R3) -> R5 eval/root runtime fixture closure
R5 -> W2-W request -> host
```

- R1/R2可立即并行；
- R3/R4等待R1；
- request/host属于后续W2-W，不由W2-R任务越界修改；
- 所有valid platform替换只改变in-memory typed identity，不改变payload bytes、cancel/timeout选择或
  diagnostic/opaque forwarding；
- 不需要用户决策或新wire/identity。

## Production owners

- R1：`runtime/capability-context/**`
- R2：`runtime/linked-type-plan/src/error.rs`
- R3：`runtime/native/**`
- R4：`runtime/service-db/**`
- R5：`runtime/eval/**`与root runtime driver eval fixtures
- W2-W：`runtime/request/**`、`runtime/host/**`

审计还确认production没有旧identity equality；exact catch equality已经在eval使用`CatchIdentity`。

