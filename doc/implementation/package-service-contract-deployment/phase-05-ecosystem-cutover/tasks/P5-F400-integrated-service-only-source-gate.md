# P5-F400 Integrated service-only source gate

状态：Ready（F395 G0，只读）。

## 直接父节点

- `P5-F395-inferred-suspension-implementation-audit-result.md`

本节点只验证N0实现base同时包含phase-05 current artifact链与Skiff main的service-only authoring。不得用
临时恢复旧`package.yml`、旧`serviceCall` marker或validator waiver制造receipt。

## 必须完成

1. 精确计算Skiff main、phase-05 integration的merge base与双向commit delta，定位service-only authoring的
   production commits/files/tests。
2. 判定当前phase-05 integration是否已经等价包含该行为，而不只比较commit ancestry。
3. 使用fresh temporary store和current Internals Relay source：
   - root没有legacy `package.yml`时能被正确分类；
   - service id/version/dependencies来自current canonical owner；
   - 生成PackageArtifact v7与ServiceContract v4 baseline；
   - 不读取stable store。
4. 负例证明缺phase-05或缺service-only任一侧都会fail closed。
5. 若当前base未满足，给出最小authoring reconciliation节点：
   - exact production/test文件；
   - 应merge/cherry-pick的commit或语义移植；
   - 冲突与测试；
   - 不修改suspension schema。

## 边界与交付

三仓production只读；不修改文件、不merge/cherry-pick、不访问stable/live/外部服务、不派子Agent。

在本任务worktree写`P5-F400-integrated-service-only-source-gate-result.md`，返回：

- `G0_PASS`及exact candidate commit/tree；或
- `TASK_SCOPE_EXPANDED`及唯一reconciliation任务。

result本地commit、worktree clean；不merge/rebase/push。
