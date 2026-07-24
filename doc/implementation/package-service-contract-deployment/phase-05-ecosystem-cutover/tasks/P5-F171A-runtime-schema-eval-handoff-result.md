# P5-F171A：Runtime Schema Eval Handoff Result

状态：Completed

## 直接父任务

- `P5-F171A-runtime-schema-eval-handoff.md`

## 交付

- eval assembly seam定义loader无关的只读`AdmittedServiceSchemaRecords`：
  `Arc<BTreeMap<PackageSchemaTypeId, Arc<PackageSchemaTypeRecord>>>`。
- Host在构造`ActiveAssemblyContextSet`时，从已经admit的`ResolvedServiceSchema`为每个精确
  `ServiceContractRef`固定record集合：
  - 校验schema声明的contract identity与store key完全一致；
  - 只克隆record `Arc`，不复制record payload；
  - 不向eval传递Package index、artifact root、resolver或文件系统能力。
- `RuntimeAssemblyEvalResolver`新增按精确contract ref读取admitted record集合的typed接口。
- internal service call与ingress target在构造时均fail closed地取得record集合，
  `RuntimeAssemblyServiceCallTarget`提供只读accessor。
- ingress额外要求请求携带的contract `Arc`与当前active assembly generation中的canonical
  contract为同一对象，拒绝跨generation contract；原有activation、operation target和request
  generation语义保持不变。
- Host execution fixture覆盖internal与ingress target共享当前generation中同一个record集合`Arc`。

## 验证

通过：

```text
rustfmt --edition 2021 <本任务修改的Rust文件>
git diff --check
```

尝试：

```text
cargo check --locked -p skiff-runtime-eval
```

该命令在本任务范围以外的后续执行断面被12个既有旧模型引用阻断，包括
`boundary_materialization`、stream与WebSocket仍引用已删除的`ContractSchemaType`、
`ContractTypeId`、`boundary_schema`和旧enum variants。编译器到达这些文件前未报告本任务
assembly seam新增接口的错误；按照任务边界，本提交未修改具体materialization、stream、
callback或WebSocket逻辑。

`cargo test -p skiff-runtime-eval assembly_seam`及Host聚焦测试同样需要先完成上述下游迁移，
目前无法形成可执行测试二进制。

## 下游断面

后续materialization、stream、callback与WebSocket任务应仅通过
`RuntimeAssemblyServiceCallTarget::schema_records()`消费当前generation已经admit的Package-owned
records，不得恢复contract-owned schema或重新读取artifact store。
