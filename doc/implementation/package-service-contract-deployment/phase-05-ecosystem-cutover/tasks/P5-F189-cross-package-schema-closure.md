# P5-F189：跨 Package Schema 传递闭包

状态：Ready

## 直接父任务

- `P5-F164-package-schema-consumer-import-result.md`

## 问题与目标

真实 `llm-providers` ServiceContract 的本包类型引用依赖 Package `agine.ai/llm-api:LlmApiFormat`，
但生成/校验 closure 缺少该跨 Package named child，导致 Relay 发布失败。修复 Package schema graph、
ServiceContract requirements 与 store resolver 的跨 Package 传递闭包，不允许 consumer 复制类型或
手工补 record。

## 验证

- 真实 llm-api→llm-providers→relay 发布链；
- 跨两层 Package child closure 正例；
- 缺失/多余/错误 owner/type id 负例；
- compiler/artifact/deployment 聚焦测试、workspace check、diff check；
- 独立提交和 result。

