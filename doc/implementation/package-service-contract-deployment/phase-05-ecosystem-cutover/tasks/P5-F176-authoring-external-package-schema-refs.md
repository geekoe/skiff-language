# P5-F176：Authoring External Package Schema Refs

状态：Ready

## 直接父任务

- `P5-F175-workspace-package-schema-test-fixtures-result.md`

## 当前断点

全workspace测试运行中，compiler authoring有两类失败：F149 JSON fixture缺当前Package schema字段；
官方std publication被`external named types cannot enter package schema v1`拒绝。

## 范围

修改compiler authoring/projection中Package schema生成规则及直接测试fixture，并写result。不得修改
artifact/runtime/deployment或consumer仓库。

## 必须实现

- 区分“当前Package拥有的named type record”和“record descriptor/API签名引用的外部Package公开
  named type”：
  - 只为当前Package声明类型生成record；
  - 允许descriptor以完整`PackageSchemaTypeRef`引用已精确解析的外部Package公开类型；
  - 不复制外部record、不改写owner、不生成当前Package-owned替身。
- 所有外部引用必须来自validated exact Package dependency（version/build/publication ABI/type id）；
  缺失或错配fail closed。
- v1禁止的是当前Package未公开的内部named type进入公开/boundary闭包，不是禁止引用外部Package公开类型。
- 修复F149 JSON fixture，使用当前完整artifact wire，不以serde default掩盖缺字段。

## 验证

- 修复当前5个失败；
- 增加跨Package公开类型引用成功及身份错配拒绝测试；
- `cargo test -p skiff-compiler --lib`；
- `cargo check --workspace`；
- `git diff --check`；
- 独立提交并写result。
