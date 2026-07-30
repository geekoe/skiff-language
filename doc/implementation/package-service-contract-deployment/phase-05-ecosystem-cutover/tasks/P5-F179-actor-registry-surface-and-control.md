# P5-F179：Actor Registry Surface 与 Control

状态：Ready

## 直接父任务

- `P5-F178-compiler-actor-nominal-handle-result.md`

## 范围

修改真实`std/actor.skiff`、Rust runtime actor native/capability/control DTO、Router actor registry/store/
protocol及其聚焦测试，并写result。不得实现actor method executor协程、升级策略或consumer service。

## 必须实现

- 真实std只公开：
  - `getOrCreate<T, Id, Bootstrap>(id, bootstrap) -> T`
  - `replace<T, Id, Bootstrap>(id, bootstrap) -> T`
  - `find<T, Id>(id) -> T?`
  - `remove<T, Id>(id) -> bool`
  删除`ActorRef<T>`、`Actor<Id>`、`put/get/ensure`。
- Rust native dispatch按compiler传入的精确actor declaration metadata编码id/bootstrap；返回内部
  `RuntimeValue::ActorRef`但boundary期望类型是名义actor T。
- control wire区分`getOrCreate`与`replace`，禁止用一个含糊put标志猜语义。
- Router store：
  - getOrCreate原子put-if-absent，已有entry时保留原bootstrap与epoch并返回现有ref；
  - replace原子写新bootstrap、推进epoch、逐出/拒绝旧incarnation；
  - find/remove保持精确identity与epoch语义。
- hard cut旧target/wire；Skiff未发布，不保留兼容别名。
- bootstrap按Actor字段shape canonical编码并绑定actor ABI/implementation事实，不接受普通opaque
  object schema identity替代。

## 验证

- Rust actor native/capability/transport/host聚焦测试；
- Router actor store/control/protocol测试与type-check；
- getOrCreate幂等、并发put-if-absent、replace推进epoch/旧ref拒绝、find/remove正负覆盖；
- 全仓公开surface无`std.actor.put/get/ensure`、`native type ActorRef`、`interface Actor`；
- `cargo check --workspace`及Router type-check；
- `git diff --check`；
- 独立提交并写result。
