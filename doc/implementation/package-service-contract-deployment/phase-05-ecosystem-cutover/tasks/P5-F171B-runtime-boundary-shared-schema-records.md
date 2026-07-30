# P5-F171B：Runtime Boundary Shared Schema Records

状态：Ready

## 直接父任务

- `P5-F171A-runtime-schema-eval-handoff-result.md`

## 当前断点

Host→eval交接使用`Arc<BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>>`保持跨contract
record去重；runtime boundary仍只接受value-owned record map。调用方若适配会在每次执行前clone
records，违背admission后不可变共享语义。

## 范围

只修改`runtime/boundary`及其聚焦测试，并写result。不得修改host、eval、loader、native或compiler。

## 必须实现

- `ServiceValuePlan`、`ServiceLinkableContractPlan`及schema closure编译接口直接接受共享record map，
  或定义同时保持`Arc<Record>`零复制的只读lookup抽象。
- 所有校验和materialization逻辑通过borrow访问record；不得clone descriptor/record来适配。
- 保持F167的owner/stable key/type id、closure、SCC、callback及fail-closed语义。
- 测试fixture改为共享records，并证明同一record Arc可被多个contract/plan复用。

## 验证

- `cargo test -p skiff-runtime-boundary`；
- `cargo check -p skiff-runtime-boundary`；
- `git diff --check`；
- 独立提交并写`P5-F171B-runtime-boundary-shared-schema-records-result.md`。
