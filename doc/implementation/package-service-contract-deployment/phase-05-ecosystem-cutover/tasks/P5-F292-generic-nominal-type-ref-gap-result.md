# P5-F292 Generic nominal TypeRef gap result

状态：共享模型缺口已确认；阻塞 F286 fully-instantiated generic nominal 条款。

## 直接父节点与权威链

- compiler consumer checkpoint：
  `P5-F291-open-error-compiler-consumer-checkpoint-result.md`
- A1 shared model：
  `P5-F284-open-error-model-acceptance-result.md`
- 唯一语言事实：
  `doc/reference/static-semantics.md` §5

## 已确认事实

1. 权威语义要求 fully-instantiated generic nominal value具有确定 runtime catch identity；runtime
   `LocalExecutionTypeIdentity`也已显式保存 type argument identities。
2. 当前 `TypeRefIr` 的 `LocalType`、`PublicationType`、`ServiceSymbol`、`PackageSymbol` 和
   `PackageSchema`引用都不携带普通 nominal instantiation arguments；只有 builtin、interface
   instantiation、call type args和 named-union concrete branch的独立字段能保存参数。
3. `compiler/lowering/src/type_lowering.rs::lower_named_type`对 local generic nominal直接报
   “not supported”；对 package/service nominal带参数路径会返回不含参数的 symbol ref。
4. `ExprIr::Construct.type_ref`、throw `payload_type`、catch type、pattern type及容器内嵌 type都复用
   `TypeRefIr`。因此只在 named-union branch DTO补参数不能表达普通 `Box<string>` value的实际 identity。
5. A1 named-union `type_arguments`只覆盖 concrete branch identity输入，不能替代 enclosing generic union
   instance或普通 generic nominal type ref。

这不是 F286 source checker可以局部猜测或从display text恢复的事实。继续实现前必须确定唯一 artifact
TypeRef表示、strict validation、link/runtime消费和 identity generation影响；不得把 generic nominal
错误降为 non-generic address，也不得暂时禁止权威语义已经允许的类型。

## 当前遮挡

F286可继续完成 non-generic CatchLeaves、declaration、site与closed-set迁移，但其 fully-instantiated
generic nominal正例及最终 A2验收必须等待本缺口关闭。runtime W2-R同样不能在缺失 type arguments的
File IR上建立正确 catch identity。

