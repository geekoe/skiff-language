# P5-F196：DB Write Callable Effect/Provenance Transfer

状态：Ready

## 直接父任务

- `P5-F184-compiler-source-regression-closure-result.md`

## 目标

修复合法 DB upsert/put/CAS 写入 detached record fields、返回 fresh status/result 的 source analyzer
transfer。当前被错误标记 unknown、escape、returnsCallerAlias、same-heap，污染 Relay 与 Registry。
按具体 DB 操作和返回形状建立精确语义，保留 caller-owned 值直接返回/逃逸的失败关闭。

## 验证

- Relay `llmProviders.chatgptPlan.importCredential` 链；
- Registry Put/PointerCas 相关链；
- detached write/fresh result 正例；
- caller alias、same heap、unknown predicate/update 负例；
- compiler source/projection、真实 Relay/Registry receipt；
- workspace check、diff check、独立提交和 result。

