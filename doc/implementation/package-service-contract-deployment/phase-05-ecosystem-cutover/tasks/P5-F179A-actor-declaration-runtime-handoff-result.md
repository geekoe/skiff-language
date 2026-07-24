# P5-F179A：Actor Declaration Runtime Handoff Result

状态：Completed

## 直接父任务

- `P5-F178-compiler-actor-nominal-handle-result.md`

## 交付

- `FileIrUnit.actorDeclarations`按声明所属文件携带canonical `ActorDeclarationIr`。Compiler从typed
  Actor声明投影精确id类型、按声明顺序排列的bootstrap字段及encoding，并计算
  `ActorAbiIdentity`。
- registry intrinsic call保留名义Actor `T0`，补齐精确`T1` id类型；只有
  `getOrCreate/replace`携带`T2` bootstrap record，`find/remove`不伪造bootstrap参数。
- File IR identity覆盖Actor声明、Actor ABI identity、id类型和字段形状；Actor id或字段变化都会改变
  文件内容身份。
- linked program保留`LinkedActorDeclaration`、字段encoding、方法ABI以及
  `LinkedActorDeclarationOwner { unit, file, actorSymbol }`。registry call只保存owner与Actor ABI
  identity引用，不复制字段或方法表。
- linker重新计算并验证Actor ABI identity，按`T0`的精确`ServiceSymbol`在同一program中解析唯一声明，
  严格校验`T1/T2`；缺声明、重名、owner不匹配、ABI篡改、id或bootstrap错配均失败关闭。

## 关键不变量

- Actor声明不进入普通`typeTable`，也不伪造record descriptor或`TypeAddr`。
- Actor的源码名义身份继续是F178规定的`ServiceSymbol`；运行时实现owner由独立
  `LinkedActorDeclarationOwner`表达。
- `ActorDeclarationIr`是id/bootstrap shape及Actor ABI的唯一事实；call site只引用，不复制声明。

## 验证

通过：

```text
cargo test -p skiff-artifact-model -p skiff-artifact-identity --no-fail-fast
# artifact-model 112 passed
# artifact-identity 75 passed; identity CLI 8 passed

cargo test -p skiff-runtime-linked-program -p skiff-runtime-linker --no-fail-fast
# linked-program 18 passed
# linker 20 passed（F179 native registry接入前的基线）

cargo check --workspace
# PASS

git diff --check
# PASS
```

新增linker负例已独立执行并通过：错误Actor id、错误bootstrap、缺失声明。canonical
`getOrCreate`正例已完成到Actor声明解析和metadata投影，随后在本任务checkpoint被下游F179尚未接入的
`std.actor.getOrCreate` runtime native binding registry拒绝；该binding owner不属于本任务。

Compiler聚焦测试同样受直接下游F179尚未替换的真实`std/actor.skiff` legacy `native type ActorRef`
阻断在prelude加载前；Compiler lowering crate check和workspace check均通过。F179合入新std surface与
native binding后应重跑这两个正例。
