# P2-T05A：Terminal PackageArtifact Projection / Emission Handoff

状态：active split；从已合入 R03/R11 的 terminal integration checkpoint 开发，与 T05 central driver
cutover 并行。

## 目标

让 compiler projection/emission 只向 `PackageArtifact` 提供 File IR、exports、implementation links、
resources、requirements 与 boundary projection，物理删除 PackageUnit/ServiceUnit/serviceAssembly 及共同
publication bundle 的 projection/emission producer。

## Ownership

- 独占 `compiler/projection/**` 与 `compiler/emission/**` 的 terminal cutover、直接 tests 和必要 crate facade。
- T05 独占 input/source/lowering/compiled/projection-input/driver/checkers；本任务不得修改这些目录。
- 保留 R03 冻结语义：export map key 是 scoped public path，Type/Const/Executable payload `symbol` 是精确
  File IR declaration symbol；不得 suffix fallback。
- 保留 R11 contract schema leaf；不修改 artifact-model、artifact-identity、compiler/contract。

## 完成态

1. PackageArtifact projection/materializer 直接消费 canonical package facts，不经 PackageUnit、runtime
   manifest、service projection 或 common publication bundle。
2. production projection/emission 中 `PackageUnit`、`ServiceUnit`、`serviceAssembly`、service publication、
   package/service option bundle producer/import 归零；旧目录/模块物理删除或改成准确终态命名。
3. package-test/runtime 下游不在本任务接线；clean-base 旧 consumer 可暂时不可用，不建 adapter。
4. 直接触碰的超长 facade 按 package-artifact model/projection/materialization/tests 拆分，不复制 identity、
   requirement、effect 或 export 规则。

## 验证

- projection/emission 聚焦 test 或最小 `cargo check`；若仅被 T05 central API 中间态阻塞，记录精确诊断，
  合流后复跑。
- R03 direct tests、targeted rustfmt、production 反向搜索、`git diff --check`。
- 不运行完整 T07 gate。

提交代码并保持 worktree clean；回报 commit、删除/重命名清单、T05 所需接口、测试与暂时 blocker。
