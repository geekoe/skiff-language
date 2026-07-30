# P5-F175：Workspace Package Schema Test Fixtures

状态：Ready

## 直接父任务

- `P5-F174-test-runner-package-schema-cutover-result.md`

## 当前断点

workspace生产代码已通过check，但`cargo test --workspace`仍逐个暴露测试专用
`PackageArtifact`/contract fixture未迁移到Package-owned schema，当前首错在
`runtime/linked-program/src/shared_image/tests.rs`。

## 范围

修改Skiff workspace内测试代码和共享测试helper，并写result。不得修改生产行为。

## 必须实现

- 迭代运行`cargo test --workspace --no-run`，修完所有测试fixture的缺失
  `package_schema_index`/`package_schema_type_records`字段及旧service-owned schema构造。
- 无public命名类型的Package使用按真实package identity计算的合法空schema。
- 有命名边界类型的fixture使用真实canonical records、index与requirements；不得用空值绕过。
- 删除测试中的`ContractTypeId`、`ContractSchemaType`、`boundary_schema`等旧模型，字符串形式的
  反序列化拒绝测试可保留但必须明确是legacy rejection。
- 不得放宽production校验来迁就fixture。

## 验证

- `cargo test --workspace --no-run`通过；
- `cargo check --workspace`通过；
- `git diff --check`；
- 独立提交并写result，列出仍可能失败但已能编译运行的既有测试。
