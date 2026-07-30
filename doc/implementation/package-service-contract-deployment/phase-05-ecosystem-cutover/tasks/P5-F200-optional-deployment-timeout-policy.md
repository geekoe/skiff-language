# P5-F200：可选 Deployment Timeout Policy

状态：Ready

## 直接父任务

- `P5-F188-internals-service-revalidation.md`

## 目标

generated deployment authoring 对未声明 timeout 的合法缺省产生 JSON null；deployment policy 当前无条件
按 u64 解析而失败。统一 optional timeout 的 canonical 表示：缺省/null 不生成 override，显式整数才
生成策略。不得要求 consumer 填虚假 timeout。

## 验证

- Account 无 timeout profile；
- 显式 timeout、零/负数/小数/字符串/额外字段矩阵；
- generated deployment round-trip 与 identity；
- deployment/compiler authoring 聚焦测试、workspace check、diff check；
- 独立提交和 result。

