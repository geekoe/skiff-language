# P5-F201：Typed Catch Tag Narrowing Provenance

状态：Ready

## 直接父任务

- `P5-F184-compiler-source-regression-closure-result.md`

## 目标

typed `catch<T>` 返回 tagged result；在 `tag == "ok"` 分支读取 `.value` 时，compiler 应保留成功
callee 的精确 type/effect/provenance，而不是降为 unknown/returnsCallerAlias。错误分支不得泄漏成功值，
未 narrowing 的 value 访问继续失败关闭。

## 验证

- Relay `migrate→catch(migrateUnsafe)→tag narrowing→value`；
- ok/error、正反比较、early return、未 narrowing、嵌套 catch；
- unknown callee 继续 unknown；
- compiler source/projection、真实 Relay receipt；
- workspace check、diff check、独立提交和 result。

