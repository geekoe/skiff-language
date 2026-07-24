# P5-F167：Runtime Package Schema Boundary

状态：Ready

## 直接父任务

- `P5-F166A-runtime-callback-package-identity-result.md`

## 同步依赖

- F166正在建立loader/admission的`ResolvedServiceSchema`；本任务只定义boundary层消费
  `PackageSchemaTypeRecord`集合的接口，合流后由后续eval任务接线。

## 范围

修改`runtime/boundary`及其聚焦测试。不得修改loader、eval、native callback、host或consumer service。

## 必须实现

- `ServiceLinkableContractPlan`、schema closure和value-plan compiler改用
  `PackageSchemaTypeId -> PackageSchemaTypeRecord`。
- `ContractTypeRef::PackageSchema`解析时严格校验package owner、stable key和type id；删除
  `Contract`、`PackagePublic`及service-owned错误类型。
- record、structural/discriminated union、representation/alias、enumeration、callback interface、nullable、
  collection/map和builtin的现有materialization语义保持。
- callback interface识别使用完整Package identity；不得clone或重建ServiceContract schema。
- 缺record、owner/key/id错配、未闭合引用和SCC fail closed。
- RuntimeTypePlan中的名义identity保留Package owner/type id，不能退化成display string。

## 验证

- `cargo test -p skiff-runtime-boundary`；
- owner/key/id任一变化拒绝；
- ordinary、stream item、typed error、callback、HTTP/WS schema-stable类型聚焦覆盖；
- `cargo check --workspace`的首错必须越过runtime/boundary并移动到后续eval/native/loader接线；
- `git diff --check`；独立提交并写result。

