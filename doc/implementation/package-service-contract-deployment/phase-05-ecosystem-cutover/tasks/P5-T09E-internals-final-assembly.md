# P5-T09E：Internals Final Environment Assembly / Workflow Closure

## 权威输入与DAG

- 设计：`/Users/geek/workspace/skiff/doc/architecture/package-service-contract-deployment.md` §3–§5、
  §9–§15。
- 依赖：T06、T07、T08、T10、T11、T12均已合入各自integration branch的exact clean commits；解锁
  cross-repo combined probe与R03。
- 风险：高；environment完整closure、actual deployment root set、共享build/reload owner。
- branch：`codex/p5-t09e-internals-final-assembly`；worktree：
  `/Users/geek/workspace/internals-p5-t09e-assembly`。
- 使用新的开发Agent；五分钟内编辑root `assembly.yml`或共享assembly workflow。任何前置deployment
  不可由canonical reader解析时，回报`TASK_NOT_EXECUTABLE`，不生成identity placeholder。

当前是Wave 3 implementation checkpoint，不是稳定候选。证据锚定三个前置repo exact commits/trees；
四对象schema/path/CLI、任一contract/package/deployment、assembly script/config变化都会使证据失效。

## 写入范围与完成态

独占Internals root `assembly.yml`、`scripts/prepare-canonical-assembly.mjs`的最终production closure、
`scripts/verify-phase05-ecosystem.mjs`、对应tests与必要root script/AGENTS接线。不得修改任何service contract/package/deployment/source、
Skiff production或`skiff-packages` source。

1. `assembly.yml`只声明environment与account、registry、Codex Relay、AIHub、Agine五个root deployment；
   resolver从exact records闭合全部package/contract/deployment，不列package source或legacy service id。
2. workflow按`publish contracts -> compile packages independently -> validate deployments -> resolve one assembly`
   执行；任何missing/tampered/duplicate provider、Host collision或partial write不移动active pointer。
3. 五个唯一Host精确为`account.skiff.localhost`、`registry.skiff.localhost`、`codex-relay.localhost`、
   `aihub.localhost`、`agine.localhost`；相同path不产生歧义。
4. `--check --no-reload`在linked worktree只写temporary immutable root并验证完整closure；`--activate`
   受primary-worktree provenance guard保护，写完records后CAS一次pointer并只reload exact generation。
5. 输出包含四类artifact identities、active environment/generation、deployment/contract/package closure与
   source provenance；不含Publication/common kind/serviceAssembly。
6. `verify-phase05-ecosystem.mjs --non-live`去重调度T08/T10/T11/T12的affected checks、registry/provider/chat
   self-tests与五deployment closure；拒绝primary/stable reload路径，供T13唯一执行。

## 唯一聚焦验证 owner

```bash
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
  node scripts/prepare-canonical-assembly.mjs --check --no-reload
node --test scripts/prepare-canonical-assembly.test.mjs
node --test scripts/verify-phase05-ecosystem.test.mjs
git diff --check
```

不运行stable build/dev/start/reload或完整gate。提交一个commit并合入Internals integration，回报root
deployment/closure/Host表、pointer不变负例、provenance与自验收矩阵。
