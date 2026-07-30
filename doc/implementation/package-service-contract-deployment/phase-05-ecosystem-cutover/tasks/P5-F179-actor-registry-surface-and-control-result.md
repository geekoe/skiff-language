# P5-F179：Actor Registry Surface 与 Control Result

状态：Completed

## 直接父任务

- `P5-F179-actor-registry-surface-and-control.md`

## 交付

- 真实`std/actor.skiff`只公开`getOrCreate/replace/find/remove`四个名义actor操作，删除公开
  `ActorRef<T>`、`Actor<Id>`及`put/get/ensure`旧surface。
- Actor native按`CallIr.actor_metadata`中的owner与ABI精确回查唯一
  `LinkedActorDeclaration`，并再次校验T0 actor、T1 id及写操作T2 bootstrap字段shape。缺失、
  伪造、歧义或ABI不一致均失败关闭；call不复制actor声明。
- actor id与bootstrap先按声明验证后的`NativeCallPlan`编码，再以`skiff-canonical-v1`
  canonical bytes传输。wire显式携带`actorAbiIdentity`、
  `actorImplementationIdentity`（当前pinned request build）和bootstrap encoding version，
  不再用普通object schema identity替代actor ABI。
- Runtime内部继续使用`RuntimeValue::ActorRef`，但native返回边界按名义actor T受控处理。
  capability、request、transport与host将`getOrCreate`和`replace`拆成独立DTO、frame及响应/错误
  路径，删除含糊`put`语义。
- Router registry/store/manager及protocol硬切为四操作：
  - `getOrCreate`在单一同步临界段执行put-if-absent；已有entry时保留首次bootstrap和epoch；
  - `replace`原子替换bootstrap及ABI/implementation facts、推进epoch并清除旧owner fence；
  - 旧incarnation按epoch拒绝；
  - `find/remove`按精确逻辑identity读取或移除当前代际。
- 修正F178遗留的跨层合同错位：`std.actor.remove`在native signature和compiler source typing中
  统一返回`bool`。

## 验证

通过：

```text
cargo test --locked -p skiff-artifact-model native_signature
# 5 passed

cargo test --locked -p skiff-compiler-source \
  explicit_actor_registry_intrinsics_return_nominal_handles
# 1 passed

cargo test --locked \
  -p skiff-runtime-native \
  -p skiff-runtime-native-contract \
  -p skiff-runtime-transport \
  -p skiff-runtime-eval \
  -p skiff-runtime-host
# eval: 85 passed
# host: 247 unit + 2 integration passed
# native: 65 passed
# native-contract: 5 passed
# transport: 70 unit + 2 integration passed

cargo test --locked -p skiff-runtime-linker actor_registry
# PASS

cargo test --locked -p skiff-runtime-eval \
  actor_declaration_owner_resolves_exact_loaded_file_and_symbol
# 1 passed

cargo check --locked --workspace
# PASS

cd router
npm run type-check
# PASS

npm test -- --run tests/actor-manager.test.ts tests/protocol.test.ts \
  tests/actor-spawn-runtime-control.test.ts
# 56 passed

npm test -- --run tests/assembly-runtime-endpoint.test.ts \
  -t "authorizes active actor/spawn control"
# 1 passed

rg '"(std\.)?actor\.(put|get|ensure)"|ActorPut|native type ActorRef|interface Actor\b' \
  std artifact-model compiler runtime router/src -g '*.{skiff,rs,ts}'
# zero matches

git diff --check
# PASS
```

Router全量suite另有不属于本任务的既有测试问题：固定在2026-05的spawn queue deadline在当前日期
已过期，以及assembly endpoint bootstrap顺序测试偶发抖动；本任务对应的聚焦路径均已单独通过。
