# P5-T13：Unique Pre-Merge Final Gate

## 角色与边界

唯一昂贵gate owner，不是开发或评审owner。依赖R03 PASS、三个repo clean exact commits/trees、无在途写入，
并持有主Agent的阶段标准→真实入口/正负例/证据/owner覆盖矩阵。

不得修改production、tests、checker、fixture、config或依赖；发现blocker立即停止verdict，退回pre-acceptance
并交由新的有界开发Agent。

## Gate 前置预检（不是PASS证据）

1. 列出 `node scripts/verify.mjs --list`、checks/ecosystem selector、外部repo tests，建立去重表。
2. 确认Node/pnpm/cargo/Mongo、router dependencies、Internals package dependencies、隔离Cargo target、动态端口。
3. 核对三仓worktree source provenance、lockfile hash、generated/store目录隔离，且任何命令不触stable
   watch registry/reload或AIHub/Agine build/dev/start。
4. 依赖安装、缓存及隔离runtime资源准备在冻结前完成并记录状态；gate只针对frozen worktree commits。

## 唯一gate ledger

对同一冻结状态执行一次：

```bash
# Skiff full non-live
pnpm --dir /Users/geek/workspace/skiff-phase-05-integration verify

# skiff-packages full non-live
npm --prefix /Users/geek/workspace/skiff-packages-phase-05-integration test

# Skiff isolated two-replica dynamic path
node /Users/geek/workspace/skiff-phase-05-integration/scripts/run-package-service-ecosystem-smoke.mjs --replicas 2

# Internals完整non-live closure与最终结果self-tests
SKIFF_ROOT=/Users/geek/workspace/skiff-phase-05-integration \
SKIFF_PACKAGES_ROOT=/Users/geek/workspace/skiff-packages-phase-05-integration \
  node /Users/geek/workspace/internals-phase-05-integration/scripts/verify-phase05-ecosystem.mjs --non-live
```

`pnpm verify`若已展开ecosystem checker，不再单独执行。`verify-phase05-ecosystem --non-live`由T09E提供，
必须从显式roots/receipts输入去重覆盖T08/T10/T11/T12的affected tests、registry history、五deployment
closure与provider/chat最终结果self-tests，不得读取repo-level `assembly.yml`或启动stable服务。

## 失败处理与输出

尽可能收集同一候选下相互独立的失败，先分类mechanical / implementation gap / owner gap /
design gap，再退回主Agent；不看到首错就修或重跑完整gate。

输出必须含三仓exact commits/trees、preflight状态、`level | command | owner | code state | result |
coverage`证据ledger、baseline/failure classification、隔离assembly/replica provenance与结果草案。
