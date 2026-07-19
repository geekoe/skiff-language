# P3-T09：Phase 03 Stable-candidate Integration Gate

## 权威输入、风险与证据状态

- 唯一架构事实源：`doc/architecture/package-service-contract-deployment.md` §2、§5、§9、§10、§11、§12、§14。
- 风险/验收组：阶段级稳定候选与唯一昂贵 gate owner。
- 当前成熟度：T02–T08只是实现检查点；完成真实链路探针且无在途 writer后才建立 stability epoch。
- evidence只对 `phase-result.md` 记录的 exact clean commit、依赖/生成物/配置与测试环境有效；任何 production
  owner、public surface、Cargo dependency、checker、fixture或 gate环境变化按影响面使证据失效。

## 角色与 DAG

- 依赖：T02–T08 全部合入 Phase 03 integration branch且无在途 writer。
- 在 `/Users/geek/workspace/skiff-package-service-phase-03` 执行；不新建 task worktree。
- 这是 Phase 03 唯一昂贵 gate owner。其它任务不得重复运行完整 selector。
- 只做机械 integration blocker修复；任何 schema/identity/projection/resolution/link/admission语义缺口退回原 owner。

## 合流与 stability epoch

1. 按 DAG而非完成时间合流：T01 → T02/T04/T05 → T03 → T06 → T07/T08。
2. 每次合流记录 source commit、target commit、冲突owner和已失效证据；合流后删除已合并 task worktree/branch。
3. 所有 production owner静止后固定 stable candidate commit；只对该 exact commit建立 evidence ledger。
4. blocker修复使相关证据失效；全部相关修复合流后建立新 epoch，不边修边反复跑完整 gate。

## 必验真实链路

至少用 canonical typed fixtures覆盖：

```text
ServiceContract + provider/consumer PackageArtifact
  -> ServiceDeployment projection
  -> RuntimeAssembly resolution
  -> typed load/hydrate
  -> shared image/link candidate
  -> whole-assembly admission/atomic swap
```

包含 A↔B service cycle、package diamond、shared build/per-activation bindings、不同 caller slot 0、empty assembly、
零/多 provider、ABI/protocol mismatch、Unavailable/missing operation、binding/template/ingress/tamper与 failed reload
preserves active等正负例。不得用 legacy aggregate builder或 fake空 contract绕过 producer。

## 唯一完整 gate

```bash
node scripts/verify.mjs --only foundation
node scripts/verify.mjs --only compiler
node scripts/verify.mjs --only runtime
node scripts/check-artifact-identity-single-source.mjs
node scripts/check-runtime-crate-dag.mjs
node scripts/check-runtime-artifact-boundaries.mjs
node scripts/check-crate-public-api.mjs --all-configured
git diff --check
```

对本阶段所有 Rust改动文件运行 targeted rustfmt。若 full workspace baseline在未改文件失败，只记录可逐字复现的
baseline，不扩大任务；本阶段改动文件不得有格式失败。

## 结果记录

新增 `phase-result.md`，写入：stable candidate commit、最终 commit、合流表、需求→代码→测试矩阵、全部命令与
exit status、失败分类/修复owner、未运行的 Phase 04/05 gate、已知残余风险。完成后提交 integration commit。
