# P5-F196：DB Write Callable Effect/Provenance Transfer

状态：Ready

## 直接父任务

- `P5-F184-compiler-source-regression-closure-result.md`

## 目标

修复合法 DB upsert/put/CAS 的 source analyzer transfer。DB transaction 必须返回真实 final expression
facts；CAS receipt 嵌入输入 candidate 时保留 nested `returnsCallerAlias`，不得虚假标记 Fresh。
DB BSON 编码只消费可静态投影字段的持久化 escape；Service boundary 是否可消费 return alias 由其
canonical value plan 单独判断。unknown predicate/update、直接持久化 caller-owned object 和后台
escape 继续失败关闭。

## 验证

- Relay `llmProviders.chatgptPlan.importCredential` 链；
- Registry Put/PointerCas 相关链；
- detached write、真实 nested return alias 及 boundary detach 正例；
- caller alias、same heap、unknown predicate/update 负例；
- compiler source/projection、真实 Relay/Registry receipt；
- workspace check、diff check、独立提交和 result。
