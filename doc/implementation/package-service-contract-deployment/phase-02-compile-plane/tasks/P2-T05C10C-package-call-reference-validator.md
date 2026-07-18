# P2-T05C10C：Canonical Package-call Reference Validator

状态：shared validation checkpoint；依赖 T05C6，T05C10D/T05C10E 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“Package direct call”“依赖与
Identity”“Fail-closed 条件”。

## 目标与 ownership

- 在 artifact-model 提供 package-call instruction 与 `ExternalRefTable.package_callables` 的唯一一致性验证。
- 独占 `artifact-model/**` validator API、遍历和直接测试；不接入 identity/emission consumer。
- 禁止修改 wire shape/version、compiler/runtime 或兼容 reader。

## 完成态

1. validator 遍历 File IR 中所有 `CallTargetIr::PackageCallable`，以
   `(PackageRefIr, PackageCallableId)` canonical key 与 table 做精确集合一致性校验。
2. missing ref、orphan ref、字段 mismatch、重复 table entry 全部 fail closed；重复 call site 可共享一个 ref。
3. service-call validator 行为不变；package-call 规则没有第二份实现。

## 验证

- artifact-model validator 聚焦测试与 crate tests、mutation matrix、targeted rustfmt、`git diff --check`。
- 不运行 compiler/runtime/T07 gate。

提交 clean checkpoint；回报 public API、错误分类与 consumer handoff。
