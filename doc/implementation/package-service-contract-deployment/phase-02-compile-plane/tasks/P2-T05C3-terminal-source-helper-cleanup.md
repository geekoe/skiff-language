# P2-T05C3：Terminal Source Helper Cleanup

状态：checkpoint repair tail；依赖 T05C2，可与 T05C/T05C4 并行，R10 前置。

权威设计：`doc/architecture/package-service-contract-deployment.md` 的“不变量”
“Compiler 与 Projection 流水线”“Fail-closed 条件”“非目标”。

## 目标与 ownership

- 清理 `compiler/source/**` 中 T05C2 后仍 orphan 的 service-publication/source helper 与直接测试。
- 独占 source 中 `runtime_type_projection::service_source`、`source_rules` 的 `collect_service_*`、
  provider native service helper、`root_refs::service_sources` 及实际同类残留。
- 禁止修改 input/input-model、core、lowering、driver、projection/emission、Cargo/checker、runtime 与
  compiler integration tests。

## 完成态

1. 每个删除 helper 均无 terminal production caller；canonical PackageSourceModel、effect/provenance、
   contract-call facts 与 R13 DB schema 路径保持不变。
2. source production 不再以 service publication/source aggregate 命名或构造旧 owner；不以 package wrapper
   或 compatibility alias 保留相同逻辑。
3. 仍有 caller 的 package/contract helper保留准确名字和直接测试；无法判断语义时报告，不猜。

## 验证

- source 聚焦测试与 `cargo check -p skiff-compiler-source`。
- source 写域 boundary/反向搜索、targeted rustfmt、`git diff --check`。
- 不运行 compiler integration tests或 T07 完整 gate。

提交并保持 worktree clean；回报删除/保留映射、production caller 证据、测试与自验收矩阵。
