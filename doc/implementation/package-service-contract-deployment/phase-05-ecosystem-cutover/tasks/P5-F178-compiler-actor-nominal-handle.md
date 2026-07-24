# P5-F178：Compiler Actor Nominal Handle

状态：Ready

## 直接父任务

- `P5-F178A-actor-declaration-artifact-checkpoint-result.md`

## 范围

修改compiler各crate、artifact-model native callable签名表及直接测试。不得修改真实
`std/actor.skiff`、Runtime actor value/transport或consumer service。

## 必须实现

- 删除所有`TypeDescriptorIr::Native`consumer、native type provider权限规则和相关fixture。
- 删除source type system对`ActorRef<T>`的已知类型、assignability、receiver和显示字符串特殊处理。
- 显式actor声明产生的名义类型可直接用于参数、返回、变量和receiver类型位置；删除
  `type implements std.actor.Actor<Id>`旧识别路径。
- actor外部值只能调用actor方法；不能普通构造、读取/写入字段、写DB或进入service boundary。
- actor方法内部`self`保持状态字段访问权。
- 建立`std.actor.getOrCreate/replace/find/remove`intrinsic typing：
  - `getOrCreate/replace`返回`T`，其bootstrap参数只在该位置按T的actor field shape校验；
  - `find`返回`T?`，`remove`不返回actor值；
  - T必须解析为actor声明，Id必须符合actor id type；
  - 删除`ensure/put/get`及`ActorRef<T>`旧surface。
  native signature可以使用独立Bootstrap类型参数承载实参，但最终关系必须由typed actor facts校验，
  不得用显示字符串猜测。
- 非actor T、错误id类型、普通构造actor及service boundary使用均有结构化拒绝测试。
- Package schema/projection不为actor类型生成record。

## 验证

- compiler workspace相关测试；
- 正例：`UserActor`变量接收getOrCreate/find结果并调用actor方法；
- 负例覆盖上述限制；
- compiler中source-level `ActorRef`、native type declaration/descriptor旧路径无命中；
- `cargo check --workspace`首错越过compiler，进入真实std/runtime actor consumer；
- `git diff --check`；
- 独立提交并写result。
