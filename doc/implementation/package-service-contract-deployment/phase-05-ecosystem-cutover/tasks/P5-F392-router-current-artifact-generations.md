# P5-F392 Router current artifact generations

状态：Ready。

## 直接父节点

- `P5-F390-compiler-router-typed-null-ping-fixture-result.md`

F390已经用fresh compiler records证明fixture本身为合法0-op/1-gateway；完整Router filesystem loader只因
冻结旧identity generation失败。本节点原子迁移current generation，不做历史dual-read。

## Worktree

- `/Users/geek/workspace/skiff-p5-f392-router-current-artifact-generations`
- branch `codex/p5-f392-router-current-artifact-generations`
- base：包含F390与本任务的Skiff phase-05 integration。

## Production要求

1. `runtimeAssemblySnapshot.ts`与`runtimeAssemblyDeploymentSnapshot.ts`只接受current
   `skiff-service-protocol-v4`，删除v3门禁，不兼容旧值。
2. `filesystemRuntimeAssemblySnapshotLoader.ts`只接受current：
   - `skiff-package-build-v8`
   - `skiff-file-ir-v8`
3. 依据PackageArtifact schema v7 / FileIR v8复核actor catalog、implementation link和record path读取，
   修正因current字段shape确实需要的相邻decode；不得放宽成unknown passthrough。
4. 使用F390未改写fresh compiler records使完整
   `FilesystemRuntimeAssemblySnapshotLoader`通过：
   - 删除test-local v4→v3 contract prefix替换；
   - 删除“current version skew应失败”的临时负例；
   - direct deployment join与full filesystem loader都消费exact records。

## 写入边界

允许：

- `router/src/router/runtimeAssemblySnapshot.ts`
- `router/src/router/runtimeAssemblyDeploymentSnapshot.ts`
- `router/src/router/filesystemRuntimeAssemblySnapshotLoader.ts`
- 上述owner的direct tests、F390两个compatibility tests。

禁止：

- compiler output、artifact schema/identity；
- F390 fixture、HTTP/WS gateway semantics；
- Host/runtime/test-runner、其它仓库、stable/live。

## 验收

至少运行：

```bash
pnpm --filter @skiff/router exec vitest run \
  tests/compilerGeneratedManifestCompatibility.test.ts \
  tests/dynamic-build-id-parity.test.ts \
  tests/filesystem-runtime-assembly-snapshot-loader.test.ts
pnpm --filter @skiff/router exec tsc --noEmit --pretty false
git diff --check
```

direct tests必须使用exact fresh `v4/v8/v8` records，不得词法改写。R0-owned/current-loader文件零type error；
全局若仍只有已记录WS HTTP-only残留，精确分类，不越界修WS。

反搜production旧`service-protocol-v3|package-build-v4|file-ir-v5`在本owner归零；若其它明确历史fixture仍
需要旧值，更新为current或报告其独立owner，不能dual-read。

写`P5-F392-router-current-artifact-generations-result.md`，production/tests/result本地commit，worktree
clean；不merge/rebase/push，不派子Agent。若current records还需要协议字段变化，返回
`TASK_SCOPE_EXPANDED`。
