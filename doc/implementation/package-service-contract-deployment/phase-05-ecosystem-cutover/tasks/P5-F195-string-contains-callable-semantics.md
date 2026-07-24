# P5-F195：String.contains Callable Semantics

状态：Ready

## 直接父任务

- `P5-F184-compiler-source-regression-closure-result.md`

## 目标

为已有 Runtime binding `receiver:string.contains@1` 建立精确 compiler callable semantics：只读 string
receiver 与参数，返回 fresh bool，不修改、逃逸、要求 same heap、调用未知目标或挂起。不得按整个
string receiver family 泛化。

## 验证

- Account `register→validEmail→string.contains` 可用；
- 精确 binding/arity/receiver 正负测试；
- compiler source/projection、真实 Account artifact/contract；
- workspace check、diff check、独立提交和 result。

