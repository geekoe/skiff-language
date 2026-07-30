# P5-F186：旧 Service-owned Schema 符号收敛

状态：Ready

## 直接父任务

- `P5-F164-package-schema-consumer-import-result.md`

## 目标

审计并清理生产代码、正向 fixture 与说明中残留的 `ContractTypeId`、`boundarySchema`、
service-owned schema definition 和旧 native type 表述。专门验证拒绝旧格式的负例可以保留，但必须
明确是 legacy rejection，不能继续作为正向 owner。

## 必须实现

- 分类每个命中：生产语义、正向 fixture、负向拒绝测试、普通英文；
- 生产和正向 fixture 统一使用 Package-owned schema records/requirements；
- 删除或改写仍生成旧 `boundarySchema` 的脚本和 cross-system fixture；
- 不机械删除负向 fail-closed 探针；
- Runtime 内部局部变量若实际承载 Package schema，改为准确命名，避免旧模型继续传播。

## 验证

- 旧符号扫描只剩明确的 legacy rejection/历史说明；
- artifact/compiler/runtime 聚焦测试；
- `cargo check --workspace`、`git diff --check`；
- 独立提交并写 result。

