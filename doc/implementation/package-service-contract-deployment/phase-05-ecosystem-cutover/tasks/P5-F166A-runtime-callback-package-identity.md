# P5-F166A：Runtime Callback Package Identity

状态：Ready

## 直接父任务

- `P5-F164-package-schema-consumer-import-result.md`

## 范围

只修改`runtime/model/src/callback_projection.rs`及其聚焦测试。

## 必须实现

- callback canonical nominal identity从`ContractTypeId`改为完整
  `PackageSchemaTypeRef`或等价的`packageId + stableSchemaKey + PackageSchemaTypeId`。
- `ContractTypeRef::PackageSchema`与local interface method的匹配不得按display string、结构碰巧相同或裸ID
  放宽；现阶段没有精确Package nominal execution mapping时应fail closed。
- builtin/record/union/nullable/literal等既有结构匹配语义保持不变。
- 删除旧`ContractTypeRef::Contract`分支和service-owned命名。

## 验证

- runtime-model聚焦测试；
- owner、stable key或type id任一不同均拒绝；
- `cargo check -p skiff-runtime-model`与`git diff --check`；
- 独立提交并写result。

