# P5-F166A：Runtime Callback Package Identity Result

状态：Completed

## 直接父任务

- `P5-F166A-runtime-callback-package-identity.md`

## 交付

- `CallbackContractProjection`的canonical nominal identity已由service-owned
  `ContractTypeId`替换为完整`PackageSchemaTypeRef`，同时保留
  `packageId + stableSchemaKey + PackageSchemaTypeId`。
- callback signature匹配删除旧`ContractTypeRef::Contract -> unknown`放宽路径。
- `InterfaceMethodType`尚未携带精确Package nominal execution mapping，因此
  `ContractTypeRef::PackageSchema`当前严格fail closed；不会按display string、结构相同或裸type id接受。
- builtin、record、union、nullable和string literal的既有结构匹配保持不变。
- 聚焦测试覆盖canonical Package身份保留，以及owner、stable key或type id不同和基准Package nominal
  均不会被`unknown`本地执行类型接受。

## 验证

通过：

```text
cargo test --offline -p skiff-runtime-model callback_projection
3 passed; 0 failed

cargo check --offline -p skiff-runtime-model
passed

rustfmt --edition 2021 runtime/model/src/callback_projection.rs
passed

git diff --check
passed
```

本任务按范围没有迁移`runtime/native`与`runtime/eval`的callback消费者；它们仍引用旧accessor，
应由后续拥有相应生产域的任务改为消费完整Package schema identity。
