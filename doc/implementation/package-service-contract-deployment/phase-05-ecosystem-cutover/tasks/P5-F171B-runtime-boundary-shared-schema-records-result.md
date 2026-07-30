# P5-F171B：Runtime Boundary Shared Schema Records Result

状态：Completed

## 直接父任务

- `P5-F171A-runtime-schema-eval-handoff-result.md`

## 交付

- boundary公开只读`ServiceSchemaRecords`类型：
  `BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>`。
  map可由Host/eval持有的外层`Arc`直接借用。
- `ServiceLinkableContractPlan`、`ServiceLinkableCapabilityRequest`和
  `ServiceValuePlan::compile`统一借用共享record map。
- schema closure校验、callback判定和value-plan compiler通过`Arc::as_ref()`读取record，
  不克隆descriptor或record payload。
- 保留Package owner/key/id一致性、递归闭包、循环检测、callback fail-closed及原有
  materialization语义。
- 测试夹具改为共享record；新增回归测试证明两个contract plan复用同一个record `Arc`，
  编译前后record引用计数不变。

## 验证

通过：

```text
cargo test -p skiff-runtime-boundary
# 172 passed; 0 failed

cargo check -p skiff-runtime-boundary
git diff --check
```

## 下游断面

Host/eval调用方应将`RuntimeAssemblyServiceCallTarget::schema_records()`解引用后直接传给
boundary API；不得重新生成value-owned record map，也不得恢复contract-owned schema。
