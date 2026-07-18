# P2-T05C6：Canonical File IR Package Call Target

状态：shared schema checkpoint；用户选择终态方案 A，T05C4/T05C7 的共同前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Package 与 PackageArtifact”
“Package direct call”“Compiler 与 Projection 流水线”“依赖与 Identity”“Fail-closed 条件”。

## 目标与 ownership

- 在 artifact-model/File IR schema 中建立唯一 package direct-call carrier：
  `CallTargetIr::PackageCallable { package_ref, package_callable_id }`。
- 建立 owner-local external ref：
  `PackageCallableRef { package_ref, package_callable_id }` 与
  `ExternalRefTable.package_callables`。
- 独占 `artifact-model` 中上述 model/schema/export/direct tests；不修改 compiler、runtime/linker、
  test-support 或 compatibility reader。

## 完成态

1. `CallTargetIr::PackageSymbol { OperationAbiRef }`、`PackageOperationSymbolRef` 与
   `package_operation_symbols` 从 terminal File IR model物理删除；不保留 serde alias或双写。
2. 新 target/ref只携带`PackageRefIr + PackageCallableId`；不重复`PackageLocalAbiIdentity`、symbol path、
   OperationAbiRef或OperationTargetRef。
3. File IR schema升级为`skiff-file-ir-v5`，format升级为`skiff-file-ir-format-v3`；opcode table不变。
4. strict serde round-trip、unknown/legacy shape rejection、default empty external-ref table与 identity/version
   direct tests通过。
5. checkpoint允许 compiler/runtime consumers暂时编译失败；不得在 schema owner中添加 adapter修复。

## 验证

- artifact-model 聚焦/全 crate tests、targeted rustfmt、旧/new wire反向搜索、`git diff --check`。
- 不运行 compiler/runtime/T07 gate。

提交并保持 worktree clean；回报 exact wire shape、version变化、consumer编译 handoff和自验收矩阵。
