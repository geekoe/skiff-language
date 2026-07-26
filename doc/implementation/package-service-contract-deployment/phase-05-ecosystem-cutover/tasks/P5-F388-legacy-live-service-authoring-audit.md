# P5-F388 Legacy live service authoring audit

状态：Ready（只读；S6精确执行边界）。

## 直接父节点

- `P5-F350-external-ingress-ecosystem-migration-audit-result.md`

父节点已冻结三个legacy root必须由一个owner统一迁移，但尚未给出每个source/package/API、40条entry及共享
harness的可直接执行清单。本节点补齐这个事实，不运行真实live。

## 审计对象

- `runtime/encrypted-storage-live/default-service`
- `runtime/encrypted-storage-live/mapped-service`
- `runtime/live-tests`
- `scripts/lib/encrypted-storage-live-harness.mjs`
- `scripts/check-db-encrypted-storage-live.mjs`
- runtime live plan/verify owner及直接测试。

## 必须回答

1. 三个root当前source模块、package依赖、collection mapping、version及service manifest事实分别应迁到哪个
   canonical `package.yml`/`api.yml`/`service.yml`字段。
2. 精确枚举40条HTTP handler route：
   - default 21、mapped 13、runtime live 6；
   - method/path/host、guard、handler、参数/返回、unary/stream；
   - 39个raw unary与1个raw server stream；
   - named key建议与gateway identity分组。
3. `runtimeKit.packageEcho`为什么需要本地private wrapper；给出wrapper的exact source owner/signature且不得
   进入API。
4. 两个encrypted root顶层guard如何映射到每个canonical gateway entry，是否需要pre/guard helper或已有
   authoring支持。
5. 三个zero-operation contract、empty operation binding、40 gateway entries/ingress的fresh publish/
   build-only验证顺序。
6. `verify-live-plan`、encrypted harness、runtime live tests哪些只需authoring receipt更新，哪些必须等用户
   明确授权的live阶段；设计一个不访问stable/live的non-live验收。
7. 判断一个实现节点是否仍合理；若文件职责/共享workflow导致应拆分，给出无重叠最小DAG。

## 边界与交付

Skiff production只读；可运行source解析、compiler dry-run或temporary fresh artifact build，但不修改文件，
不连接stable/live/Mongo/外部服务，不派子Agent。

在本任务worktree写
`P5-F388-legacy-live-service-authoring-audit-result.md`，包含：

- exact文件清单与route矩阵；
- canonical manifest示例；
- wrapper/guard owner；
- production与non-live测试边界；
- 是否需要用户决策。

result本地commit、worktree clean；不merge/rebase/push。
