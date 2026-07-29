# P5-F445H I7 P8 J Final hermetic gate

状态：

```text
BLOCKED_BY = X_PASS
GATE_OWNER_UNIQUE = YES
```

## 1. Inputs and freeze

- 直接父节点：
  `P5-F445H-I7-P8-X-independent-http-entry-acceptance.md`
- ancestry floors：
  - Skiff `3a87d37f81a04c249f308b311bd91dcfdf3a8aa3` /
    `eafc29e952f6b5170e4f5faca4e5d181b3ace9f6`
  - Internals `9c3bdc82c4a43e575ea627357c05f54dbc0400a8` /
    `c3f159a397cd3c2b316a502ce945d8a935a9c2c3`
- gate执行前必须记录X PASS的精确两个repo commit/tree、命令来源、缓存/临时Mongo/端口状态；候选变化
  结束当前stability epoch。

## 2. Gate

唯一gate owner按最终候选运行：

1. P8 T真实HTTP entry combined probe；
2. AIHub默认51个non-live tests：目标`51 pass / 0 fail / 0 skip`；
3. Agine默认non-live tests（当前基线目标170，实际发现数必须记录且不能静默下降）；
4. Codex Relay全部default isolated tests；gate前只读发现当前默认test files/cases并冻结数量，最终实际
   发现数不得下降，使用canonical `scripts/test-isolated-service.mjs agine.ai/codex-relay`入口；
5. official `skiff-packages`全部default offline tests；先用
   `node scripts/test-packages.mjs --all --list`冻结offline计划，再以相同Skiff候选运行
   `node scripts/test-packages.mjs --all`，default发现数不得下降，live/manual tests不执行；
6. Account没有explicit test service，不伪造default isolated target。保留现有
   `skiff-platform/account/service-api-receipt.test.mjs`以及canonical assembly/graph checks；
7. 受P8 production写集影响的Skiff component selector，包括S1实际修改的Runtime/Host表面；若R为
   NO-OP且H/K聚焦证据仍有效，不重复无关full workspace。

AIHub可使用用户已授权的临时managed Mongo，只能动态端口、临时目录、sanitized env并在结束后清理。
禁止stable instance、外网、OAuth、browser、真实API key和`defaultRun false` live test。任何skip、零发现、
遗留进程/端口或未清理临时状态均不是PASS。

结果ledger采用：

```text
层级 | 命令 | owner | commit/代码状态 | 结果 | 覆盖范围
```

J不修改候选。失败先分类并退回预验收，不在gate状态顺手修复或连续重跑完整矩阵。

AIHub、Agine、Codex Relay、official packages与互不重叠的Skiff selectors可以在冻结candidate上并行，
但必须先由gate owner按磁盘余量、独立`CARGO_TARGET_DIR`/artifact root、端口lease和managed Mongo/
isolated stack资源做前置分组。共享Cargo target、同一隔离stack owner或空间不足的分片必须串行，不能
为了并行复用可变缓存或共享服务。任一分片失败只收集同一candidate的诊断与其它独立失败；不在运行中的
gate边修边重跑。

## 3. Early diagnostic wave and final acceptance

为避免只在关键路径末尾发现独立consumer问题，S1开发与I恢复期间可以由各自唯一owner在当时精确的
pre-acceptance candidate上并行运行Codex Relay、official packages或其它尚未覆盖的独立矩阵。该轮只用于
提前发现和归类blocker，必须记录精确commit/tree，不能成为J PASS证据，也不能由gate分片边跑边修。

所有diagnostic blocker批量闭合、S1/I合流、X PASS且没有在途写入后，才冻结final candidate并执行本文件
第2节的最终验收。任何影响这些consumer的后续代码、配置、依赖或环境变化都会使早期诊断证据失效；最终
candidate仍须由J唯一owner建立一次对应的验收结果。
