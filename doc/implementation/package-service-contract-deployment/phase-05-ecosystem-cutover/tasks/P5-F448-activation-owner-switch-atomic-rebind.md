# P5-F448 Activation Owner Switch Atomic Rebind

状态：**READY / IMPLEMENTATION ASSIGNED / R448 PENDING**

## Authority

- [`package-service-contract-deployment.md`](../../../../architecture/package-service-contract-deployment.md)
  §6.2
- [`runtime-deployment-topology.md`](../../../../architecture/runtime-deployment-topology.md)
  “Activation execution owner switch”
- [`runtime-layered-crate-architecture.md`](../../../../architecture/runtime-layered-crate-architecture.md)
  `ActiveAssemblyContextSet`与`ActivationExecutionContextRebinder`
- [`runtime.md`](../../../../reference/runtime.md) “Service call 的 activation owner”
- [`P5-F446C-activation-runtime-service-db.md`](P5-F446C-activation-runtime-service-db.md)

本文只拆实现owner、集成顺序和证据，不新增第二套语义。

## Finding

F446已经让每个generation精确钉住assembly/config snapshot，也已经形成deployment-scoped ConfigView和
service DB。但当前service provider进入链若只替换service identity或binding，会让provider继续使用caller
的config、DB、file、actor、spawn、WebSocket或telemetry owner；若反过来整体替换request context，又会
丢失原request deadline、内部停止、trace、stream/test lifecycle或错误关联。

正确边界是一次generation-pinned原子rebind：

- deployment-scoped owner全部来自target deployment；
- request-scoped owner全部继承source request；
- provider使用fresh heap并做boundary materialization；
- missing exact context直接失败，不查latest；
- service provider和callback owner是仅有的rebind入口。

## Implementation Ownership

### Activation/host owner

- 建立exact `ActiveAssemblyContextSet`，同时保存active及仍有pin的draining generation；
- key包含environment、assembly ref、config snapshot ref、generation与deployment ref；
- 实现`ActivationExecutionContextRebinder`的validate-then-publish原子构造；
- target只来自validated service binding或callback capability；
- 投影provider的config、DB、file、actor capability/registry、spawn、WebSocket、telemetry及service
  dependency context；
- missing、duplicate、cross-generation或cross-pair target全部fail closed，无latest/ambient fallback。

### Boundary/eval owner

- service provider entry与callback owner entry接入同一rebinder，删除其它手写部分切换；
- provider创建fresh heap，参数、返回、error payload、callback payload与stream item使用contract value
  plan materialize；
- 继承deadline、内部停止/cancellation、time source、request generation/lifecycle、trace/error、
  transport request identity、stream lifecycle、test effect/case capability与heap limits；
- caller call frame、slot、mutable root和`ActorExecutionFrame`不得进入provider；
- service call实际Pending时仍由caller actor suspension owner释放/恢复executor。

### Lifecycle/regression owner

- Package静态资源继续随`RuntimeExecutionProjection`和current callable Package owner，不进入activation
  rebind字段；
- 内部`ActorRef`显式route owner在rebind前后不变；
- escaping service stream持有创建时exact generation pin，active generation切换后仍从旧provider
  context产生item/callback/terminal，结束后才释放；
- 普通continuation、Package direct call、actor resume、spawned request start与native helper不能成为
  第三种rebind入口。

三个owner可以在同一shared checkpoint后并行，但不得各自定义context字段清单或复制rebinder。共享类型先由
Activation/host owner落短checkpoint；其它owner只消费。

## Integration And Evidence

- 实现Agent在自己的worktree只做静态检查、测试代码审阅与`git diff --check`，不运行Rust编译；所有候选合入
  单一integration branch后，由唯一gate owner编译并运行受影响Runtime聚焦测试一次；
- integration必须包含active/draining两generation同service不同owner的动态fixture；
- 必须有provider owner全字段矩阵、request owner全字段矩阵、fresh heap/cross-heap materialization、
  callback roundtrip、ActorRef/actor frame、static resource和escaping stream证据；
- 反向搜索不得存在按service ID/latest generation查provider、thread-local current service、provider直接
  复用caller heap或第三个rebinder入口；
- 通过[`P5-R448-activation-owner-switch-acceptance.md`](P5-R448-activation-owner-switch-acceptance.md)
  的独立验收前，不得把F446/R446写为完成。

## Out Of Scope

- 不改变ServiceContract、value plan、ActorRef源码surface、spawn语义或Package静态资源identity；
- 不新增remote boundary、跨generation迁移、public cancel API或stream自动升级；
- 不保留旧部分切换、latest fallback或双路径。
