# P5-F199：Bytes.fromBase64 Callable Semantics

状态：Ready

## 直接父任务

- `P5-F195-string-contains-callable-semantics-result.md`

## 目标

为 `core.bytes.fromBase64` 建立与已有 native signature/Runtime handler 一致的精确 callable
semantics：读取 string，返回 fresh bytes，无 caller mutation/escape/same-heap/unknown/suspend。
非法 Base64 的既有 typed/runtime error 行为保持。

## 验证

- Relay `codec.jwtPayload→claimsFromJwt→importCredential` 链不再 unknown；
- exact binding、输入、返回及错误正负测试；
- artifact/source/projection、真实 llm-providers/Relay receipt；
- workspace check、diff check、独立提交和 result。

