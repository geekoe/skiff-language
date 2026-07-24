# P5-F181：Compiler-owned Builtin Prelude

状态：Ready

## 直接父任务

- `P5-F178-compiler-actor-nominal-handle-result.md`

## 当前断点

F178删除`prelude/*.skiff`中的`native type`伪声明后，compiler prelude registry仍依赖这些声明发现
`bytes`、`Array`等模块内类型，导致官方std authoring报unknown standard_library type。

## 范围

修改compiler builtin/prelude registry、platform source validation和直接测试；不得恢复`native type`
源码声明，不得修改runtime/router/actor executor。

## 必须实现

- compiler-owned registry显式拥有完整builtin闭集及其可见模块/别名：
  `bytes`、`Array`、`Map`、`Config`、`Date`、`Json`、`JsonObject`、`Stream`、Exception/error
  builtins、session capability builtins等当前prelude实际使用集合。
- builtin identity/kind/type arity不从`.skiff`声明推导；prelude文件只提供函数/方法surface。
- 模块内引用、root/prelude别名和官方std source validation使用同一registry事实。
- 未知builtin、错误arity、用户伪造同名声明fail closed。
- 不把builtin投影成普通record或Package schema named record。

## 验证

- 修复当前compiler lib 5个失败；
- prelude registry/source/std authoring聚焦测试；
- 全prelude无`native type`；
- `cargo test -p skiff-compiler --lib`；
- `cargo check --workspace`；
- `git diff --check`；
- 独立提交并写result。
