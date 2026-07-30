# P5-F186：旧 Service-owned Schema 符号收敛 Result

状态：Completed

> Authority note（2026-07-29）：本result如实保留当时证据；下文把`assembly.yml`列为正向authoring面的
> 结论已失效。RuntimeAssembly现由operational roots生成，不存在developer-authored assembly manifest。

## 直接父任务

- `P5-F186-legacy-schema-symbol-closure.md`

## 交付

- 按生产语义、正向 fixture、负向拒绝和历史说明分类旧符号：
  - 生产代码与正向 fixture 不再使用 `ContractTypeId`、`boundarySchema`、
    `ServiceSchemaRecords` 或 `boundary_schema`；
  - `ContractTypeId` 只剩权威架构文档中明确禁止 service-owned identity 的历史对照；
  - `native type` 源码与 wire 命中只保留明确的 legacy rejection；recoverable native adapter
    identity 和 native callable type argument 属于仍在使用的独立语义，未机械删除。
- 删除两份已经没有测试入口、仍以 service-owned schema 为正向 owner 的旧 Rust fixture：
  `compiler/contract/src/tests/schema_fidelity.rs` 和
  `compiler/projection/src/package_artifact/tests/boundary.rs`。
- Runtime 将实际承载 Package-owned records 的模块、类型和变量统一改为
  `package_schema_records` / `PackageSchemaRecords` / `AdmittedPackageSchemaRecords`，service
  只负责选择被 admission 接受的精确 record 集合。
- compiler builtin registry 删除遗留的 `native_type_names` 命名，统一为
  `builtin_type_names`；Runtime builtin fixture 和 artifact helper 同步使用 builtin 表述。
- cross-system checkpoint 改为当前唯一正向 authoring 面：
  `package.yml services[]`、`service.yml`、`config.<profile>.yml` 和 `assembly.yml`，不再列出
  开发者维护的 `contract.yml` / `deployment.yml`。
- 删除仍直接构造独立 contract/deployment 的退役 tooling 测试及其专用 helper；保留并验证
  独立 contract/deployment authoring 必须 fail closed 的负例。

## 旧符号扫描

生产代码与正向 fixture 的以下命中为零：

```text
ContractTypeId
boundarySchema
boundary_schema
ServiceSchemaRecords
service_schema_records
```

剩余 `native type` 命中均属于：

- 删除语法、AST 或旧 wire 的 fail-closed 负例；
- 权威文档对已删除 source/native type 模型的明确说明；
- recoverable native adapter identity 或 native callable type argument，不是 source type owner。

## 验证

通过：

```text
cargo test --offline -q \
  -p skiff-artifact-model \
  -p skiff-compiler-contract \
  -p skiff-compiler-projection \
  -p skiff-compiler-source \
  -p skiff-runtime-boundary \
  -p skiff-runtime-eval

artifact-model 114 passed
compiler-contract 1 passed
compiler-projection 22 passed
compiler-source 230 passed
runtime-boundary 172 passed
runtime-eval 118 passed

cargo test --offline -q -p skiff-runtime-linked-type-plan
11 passed

node --test scripts/tests/package-service-authoring.test.mjs
9 passed

node cross-system-fixtures/package-service-ecosystem/verify.mjs --self-test
ok; controls=6; rawCases=78

cargo check --workspace
passed

git diff --check
passed
```

`skiff-runtime-native` 在本任务分支与未修改的 integration 基线均为 `60 passed; 5 failed`；
五项失败都来自既存 native callable semantics registry 与
`std.string.truncateUtf8Bytes` / capability audit 不一致，不由本任务的 schema 命名收敛引入。
