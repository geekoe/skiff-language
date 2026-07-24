# P5-H34：Actor Type 与 Builtin IR Cutover

状态：Design Complete / Implementation Ready

## 直接父任务

- `P5-H32-package-owned-type-cutover.md`

## 用户决策

- actor自定义类型与其他自定义类型一样，声明后直接用于类型位置。
- `UserActor`本身表示对具体`UserActor`实例的类型安全句柄；源码不再使用
  `ActorRef<UserActor>`。
- Runtime内部可保留`ActorRef`实现结构，但它不是source/artifact公开类型。
- 当前真实源码中除`ActorRef<T>`外没有opaque `native type`使用者，因此删除`native type`
  语言功能，不保留未使用扩展点。
- `native function`保留。
- 删除`TypeDescriptorIr::Native`；将`TypeRefIr::Native`收窄并重命名为`Builtin`。
- 将native callable签名中的`NativeTypeExprDef`改为不与opaque native type混淆的
  `NativeSignatureTypeExpr`。

## 语义合同

- actor句柄只能调用actor方法；外部不能读取actor字段、普通构造、按普通值复制、写入DB或进入
  service boundary payload。
- actor方法内部的`self`仍拥有actor状态访问权。
- actor manager操作返回actor声明类型本身；编译器必须证明类型参数/目标确为actor，不靠
  `ActorRef<T>`包装提供证明。
- builtin是语言闭集，每种builtin分别定义assignability、runtime layout和boundary能力；
  `Builtin`不意味着一律可序列化。例如`string`可作为普通boundary value，`Stream<T>`只允许在
  专门stream位置。
- Package schema只为具有合法boundary descriptor的公开named type生成record；actor类型与内部
  stream handle不得进入普通service payload。

## 实现顺序

1. shared artifact/syntax checkpoint：删除`native type`声明模型，完成Builtin及native signature
   类型重命名，更新严格wire。
2. compiler source/input/projection/lowering：actor名义类型直接流转，删除ActorRef特殊source
   wrapper与native descriptor路径。
3. std/runtime：删除`std/actor.skiff`中的`native type ActorRef<T>`，actor manager签名返回actor
   名义类型；Runtime内部值和路由继续保留精确actor type/id。
4. 全仓fixture、golden、Package schema与service boundary拒绝测试。
5. 恢复F176的external Package schema ref修复，再运行workspace与三仓库验收。

## Gate

- 全仓源码/公开artifact无`ActorRef<T>`与`native type`；
- `TypeRefIr::Native`、`TypeDescriptorIr::Native`、`NativeTypeExprDef`无残留；
- 普通actor类型变量可接收actor manager结果并调用actor方法；
- 非actor类型不能传给actor manager intrinsic；
- actor类型不能进入service payload或DB；
- native function与builtin签名回归通过；
- `cargo test --workspace`及真实service gates通过。
