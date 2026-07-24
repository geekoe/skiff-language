# P5-F169：Runtime Callback Package Schema Cutover

状态：Ready

## 直接父任务

- `P5-F167-runtime-package-schema-boundary-result.md`

## 当前断点

runtime boundary已经只接受`PackageSchemaTypeId -> PackageSchemaTypeRecord`，并以完整Package
identity识别callback interface；但`runtime/native/callback_adapter.rs`和
`runtime/eval/assembly_execution/callback_native.rs`仍保存旧`ContractTypeId`及
`ContractSchemaType`集合，无法连接新的boundary计划。

## 范围

修改`runtime/native`和`runtime/eval`中的callback native/adapter接线及聚焦测试，并写本任务
result。不得修改artifact model、compiler、loader、boundary、host或consumer service。

## 必须实现

- callback adapter的名义身份改为完整`PackageSchemaTypeRef`，schema输入改为admission阶段解析出的
  `PackageSchemaTypeId -> PackageSchemaTypeRecord`只读集合。
- eval callback native直接消费已经解析和校验的Package records以及boundary产生的callback计划；
  不得从service artifact、Package index或文件系统再次解析schema。
- callback operation、参数、返回值、typed error及嵌套命名类型解析保留Package owner、stable key、
  type id三元身份；不得退化为显示名称或仅type id。
- adapter缓存键必须包含完整Package名义身份；相同短名称或stable key但不同Package的callback不得
  混用。
- 删除旧`ContractTypeId`、`ContractSchemaType`、service-owned boundary schema accessor和任何
  临时双模型兼容分支。
- 缺record、owner/key/id不匹配、非callback descriptor、未闭合引用必须fail closed；不得panic，
  不得延迟到实际callback调用后才发现。
- callback handle生命周期、取消、一次性/多次调用语义和现有错误传播保持不变。

## 验证

- `cargo test -p skiff-runtime-native`；
- `cargo test -p skiff-runtime-eval callback`（若过滤不足以覆盖相关测试，再跑完整eval测试）；
- 增加至少以下聚焦覆盖：完整Package identity成功、同名跨Package隔离、owner/key/id任一错配拒绝、
  非callback类型拒绝、缺传递record拒绝；
- `rg "ContractTypeId|ContractSchemaType" runtime/native/src/callback_adapter.rs
  runtime/eval/src/assembly_execution/callback_native.rs`无命中；
- `cargo check --workspace`的首错越过上述callback文件；
- `git diff --check`；
- 独立提交并写`P5-F169-runtime-callback-package-schema-cutover-result.md`。
