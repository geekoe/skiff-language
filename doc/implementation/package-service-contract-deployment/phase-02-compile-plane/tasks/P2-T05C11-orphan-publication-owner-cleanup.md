# P2-T05C11：Orphan Publication Owner Cleanup

状态：A01 P0 blocker；依赖最终 T07 candidate，T05C12 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“四对象模型”“Package 编译”
“Compiler 与 Projection 流水线”“发布、存储与平台模型”“Fail-closed 条件”。

## 目标与 ownership

- 物理删除 canonical facade 已断链、但仍公开存在的 publication-era package aggregate 与 projection adapter。
- 独占 `compiler/compiled/**`、`compiler/projection-input/**` 及其直接 tests/Cargo cleanup。
- 禁止修改 driver canonical facade、projection/emission、checker、runtime 或新增兼容 caller。

## 完成态

1. `PackagePublication`/`PackagePublicationInfo`、publication dependency metadata/config/collection mapping DTO、
   `PackageProjectionInput` 与 publication→projection adapter 物理归零，不以改名/私有 dead code保留。
2. source/compiled facts 只接受 canonical `CompiledPackage`/当前 source facts，不再接受 publication 集合。
3. 唯一 handoff 是 `CompiledPackage -> ProjectionInput`；projection input 不拥有 manifest/config/collection mapping
   或 service/deployment metadata。
4. 删除孤儿 API 后受影响 crates/compiler 可编译，直接 tests使用终态类型；无 dual path/fallback。

## 验证

- impacted crate tests/check、public API轻量探针、旧 aggregate/field/adapter反向搜索、targeted rustfmt、
  `git diff --check`；不跑完整 foundation/compiler/T07 gate。

提交 clean checkpoint；回报删除 API、保留 handoff、测试与下游 blocker。
