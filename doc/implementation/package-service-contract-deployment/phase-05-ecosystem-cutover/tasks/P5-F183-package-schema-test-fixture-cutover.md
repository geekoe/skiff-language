# P5-F183：Package Schema 测试夹具硬切

状态：Ready

## 直接父任务

- `P5-F164-package-schema-consumer-import-result.md`

## 问题

全量 workspace 测试中，大量旧夹具声明 exact Package/std requirement，却没有提供 F164 后要求的
已验证 public schema bundle 或 canonical store resolver，因而在真正被测逻辑之前统一失败：

`exact package requirement ... has no resolved schema or canonical store resolver`

另有少量夹具仍携带多余 schema record、旧 build identity golden 或旧错误文本。

## 目标

把测试基础设施和夹具迁移到唯一的 Package-owned schema 输入链，不在 production code 中恢复
manifest/source fallback，也不放宽缺失 schema 的失败关闭语义。

## 范围

- compiler/test support 与 compiler integration tests；
- test-runner fixtures/tests；
- runtime/package-test 的相关 fixture；
- 必要的共享 test helper。

不得修改 Package schema production trust boundary 来迁就旧夹具。

## 必须实现

- 为需要依赖 Package/std schema 的测试构造 canonical、已验证的 schema bundle/store；
- 共享 helper 必须从真实 Package public declarations/records 构造，不能手写与 production 不一致的
  descriptor 副本；
- 无 schema 需求的依赖不得被迫生成虚假 schema；
- 缺 bundle/store 的既有负例继续失败关闭；
- 修正 exact closure，删除多余 record；
- 只在输入语义确已变化时刷新 build identity golden，并保留“无关 Package 类型不改变 service
  protocol identity”的断言；
- 更新因新精确校验导致的错误文本断言，不弱化错误类型。

## 验证

- 全量失败清单中所有 `no resolved schema or canonical store resolver` 均消失；
- compiler integration tests、runtime/package-test、test-runner 聚焦测试；
- 缺 schema、篡改 record/index、错误 Package ID/build/ABI 的负例继续通过；
- `cargo check --workspace`、`git diff --check`；
- 独立提交并写 `P5-F183-package-schema-test-fixture-cutover-result.md`。

