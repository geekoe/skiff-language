# P5-F198：JsonObject.has Callable Semantics

状态：Ready

## 直接父任务

- `P5-F195-string-contains-callable-semantics-result.md`

## 目标

为已有 Runtime binding `receiver:JsonObject.has@1` 建立精确 compiler callable semantics：只读
JsonObject receiver 与 string field，返回 fresh bool，无 mutation/escape/same-heap/unknown/suspend。
不得泛化整个 JsonObject receiver family。

## 验证

- Account `verifyDomainChallenge→jsonObjectField→value.has(field)` Available；
- binding/receiver/arity/type 正负测试；
- compiler source/projection、真实 Account 20 operations；
- workspace check、diff check、独立提交和 result。

