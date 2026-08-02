# F5：router `inMemoryRegistryStore` idle 逐出与 upgrade 互卡竞态修复

## 引用链

- 权威设计：`doc/architecture/actor-shared-heap-design.md`（main `40fac3b6`）§7.2
  “前置修复（独立于压缩）”：进入 `upgrading` 时未取消 pending idle eviction，
  逐出 ACK 清 owner 后 `upgradeFence` 永久等待；压缩/升级优先级成立前必须先修复
  状态机并补 router 回归测试。
- 直接父节点：`doc/implementation/actor-shared-heap/interfaces.md`（main
  `40fac3b6`）§8 Router（F5，独立）：进入 `upgrading` 时取消/清理 pending idle
  eviction；upgrade 完成容忍 owner 丢失；补 router 回归测试。无跨模块接口依赖。
- 集成目标：`/root/integration_actor_shared_heap`（集成分支
  `integration/actor-shared-heap`，基线 `14c06b8c`）。

## 任务范围

- 写集：`router/src/actor/inMemoryRegistryStore.ts`、`router/tests/actor-owner-lease-idle-ttl.test.ts`、
  本任务文件。
- 禁止：runtime/compiler、artifact-model、`.github/workflows`、其他 router 文件。
- 基线：`14c06b8cb6c18b6182dfcb3842f82fa7245d2b37`（integration/actor-shared-heap）。

## 修复方案（对齐 §8，最小闭合）

1. `admitActorMethod` 的 mismatched-implementation 分支进入 `upgrading` 时，
   同步清理 `idleEvictionRequestId` / `idleEvictionRequestedAt`（取消 in-flight
   idle eviction），并快照旧 owner 身份（store 私有 map）供 upgrade fence 使用。
2. `acknowledgeIdleOwnerEviction` 增加 `lifecycleState === 'live'` 前置条件：
   逐出 ACK 永远不能清掉 `upgrading` 条目的 owner（防御性闭合）。
3. `upgradeFence` / `upgradeFenceMatches` 在 entry owner 已丢失时回退到快照，
   使 `actorUpgradeFence`、`waitForActorUpgradeDrain`、`completeActorUpgrade`
   在 owner 丢失后仍可工作；升级完成后清理快照。
4. 快照清理路径：`writeBootstrap`、`remove`、`acquireOwnerLease` 成功、
   `evictIdleActor`、`completeActorUpgrade` 成功、`finalizeRemoveIfIdle`。

## 回归测试

`router/tests/actor-owner-lease-idle-ttl.test.ts` 新增两个用例：

1. in-flight idle eviction + mismatched-implementation 翻转到 `upgrading`；
   ACK 必须返回 `false`（取消生效），entry 保留 owner，upgrade fence 可用，
   drain + complete 成功，随后 v2 重新 acquire 并 admit 成功。
2. `upgrading` 期间 owner 丢失（disconnect）后，fence 仍可由
   `actorUpgradeFence` 取得，drain + complete 成功，entry 到达一致状态
   （epoch+1、v2、inactive、无 owner），随后 get 成功。

两个用例在修复前基线均失败（用例 1 在 ACK 断言失败；用例 2 在 fence 断言失败）。

## 验证命令

```bash
cd /Users/geek/workspace/skiff-actor-f5/router
npm install   # 无 package-lock；只装 vitest/tsc 依赖
npm test -- tests/actor-owner-lease-idle-ttl.test.ts \
  tests/actor-router-admission.test.ts \
  tests/actor-get-create-activation.test.ts
npm run type-check
```

## 自验收

按主流程“设计/任务条款 | 代码证据 | 反向搜索证据 | 测试”矩阵，报告给
`/root/integration_actor_shared_heap` 与 `/root`。
