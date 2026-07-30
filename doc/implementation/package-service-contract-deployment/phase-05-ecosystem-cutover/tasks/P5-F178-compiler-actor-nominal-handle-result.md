# P5-F178：Compiler Actor Nominal Handle Result

状态：Completed

## 直接父任务

- `P5-F178-compiler-actor-nominal-handle.md`

## 交付

- compiler type resolution建立独立actor nominal fact；显式`actor`声明可直接用于参数、返回、
  变量和receiver类型位置，不再通过`std.actor.Actor<Id>` conformance识别actor。
- actor fact精确保留id type和bootstrap field shape。本地actor也使用名义
  `ServiceSymbol`身份，不投影成record shape；普通record构造、外部字段访问、DB使用和boundary
  record projection均失败关闭。actor方法内部`self`可读取声明的状态字段。
- expression typing新增`std.actor.getOrCreate/replace/find/remove` intrinsic：
  `getOrCreate/replace`返回`T`并仅在bootstrap参数位置按actor field shape做上下文校验，
  `find`返回`T?`，`remove`返回`void`；T、id和bootstrap均由typed actor facts校验。
- actor receiver只解析actor自身impl方法。compiler/lowering删除`ActorRef<T>` receiver/spawn
  特殊路径；semantic删除旧Actor接口known conformance路径；storage projection不再从旧
  conformance伪造Runtime actor metadata。
- artifact-model native signature表加入四个actor registry binding，使用独立Bootstrap类型参数；
  compiler prelude fixture删除native type声明并切换到四函数surface。真实`std/actor.skiff`和
  Runtime actor value/transport均未修改。
- Package schema/projection看不到actor record shape，因此不会为actor生成record schema。

## 验证

通过：

```text
cargo test -p skiff-compiler-source actor -- --nocapture
# 3 passed

cargo test -p skiff-artifact-model native_signature -- --nocapture
# 5 passed

cargo check -p skiff-compiler-lowering -p skiff-compiler-source \
  -p skiff-artifact-model

cargo check --workspace

git diff --check
```

正例覆盖`UserActor`接收`getOrCreate`/`find`结果、调用actor方法及方法内部读取`self`状态。
负例覆盖非actor T、错误id、错误bootstrap、普通构造、外部字段访问和DB使用。workspace check
在合入P5-F178B后完整通过。
