# P5-F445H I7 P8 J Final hermetic gate

状态：

```text
BLOCKED_BY = X_PASS + A1_PASS
GATE_OWNER_UNIQUE = YES
```

## 1. Inputs and freeze

- 直接父节点：
  - stream lane：`P5-F445H-I7-P8-X-independent-http-entry-acceptance.md`
  - Agine compiler lane：`P5-F445H-I7-P8-A1-top-level-alias-instance-method-closure.md`
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
3. Agine默认non-live tests（当前源码声明基线目标170，实际发现数必须记录且不能静默下降）。Early
   diagnostic在Skiff `2bcb40e61ee6b922eeca913651e2cc344a38b50e`上得到`170 declared /
   0 discovered`，阻塞是A1拥有的`topLevelAlias`精确package receiver method编译缺口；A1合流前不能把
   零发现写成stream、runner或Agine业务失败，A1 PASS也不能替代本项最终170个测试；
4. Codex Relay全部default isolated tests；gate前只读发现当前默认test files/cases并冻结数量，最终实际
   发现数不得下降，使用canonical `scripts/test-isolated-service.mjs agine.ai/codex-relay`入口；
5. official `skiff-packages`全部default offline tests；先用
   `node scripts/test-packages.mjs --all --list`冻结offline计划，再以相同Skiff候选运行
   `node scripts/test-packages.mjs --all`，default发现数不得下降，live/manual tests不执行；
6. Account没有explicit test service，不伪造default isolated target。保留现有
   `skiff-platform/account/service-api-receipt.test.mjs`以及canonical assembly/graph checks；
7. 受P8 production写集影响的Skiff component selector，包括S1 diagnostic fixture与S2/S3实际修改的
   Runtime/Host表面；S1没有production改动。若S2/S3为NO-OP、R为NO-OP且H/K聚焦证据仍有效，不重复
   无关full workspace。

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

为避免只在关键路径末尾发现独立consumer问题，S2/S3开发与I恢复期间可以由各自唯一owner在当时精确的
pre-acceptance candidate上并行运行Codex Relay、official packages或其它尚未覆盖的独立矩阵。该轮只用于
提前发现和归类blocker，必须记录精确commit/tree，不能成为J PASS证据，也不能由gate分片边跑边修。

当前early diagnostic已经把Agine blocker固化到
`P5-F445H-I7-P8-D2-agine-top-level-receiver-authority-result.md`，其实现子节点A1只修改Compiler
projection/source/lowering，不属于P8 stream根因或S2/S3/I修复面。两条lane可以独立推进：

```text
P8 stream lane:   S1 diagnostic -> S2 -> S3 -> I -> X ---+
                                                          +-> J final gate
Agine compiler:   D2 -> A1 -> Agine 170 resume -----------+
```

S1保持`TASK_NOT_EXECUTABLE / S1_COMPLETE=NO`；它的GREEN fixture只是否定普通return-stream根因。
`P5-F445H-I7-P8-D3-stream-argument-response-sink-refinement-result.md`把后继拆成顺序S2/S3，禁止把
argument transport和response sink混在一次修复中。

所有diagnostic blocker批量闭合、S2/S3/I合流、X PASS、A1 PASS且没有在途写入后，才冻结final candidate
并执行本文件第2节的最终验收。任何影响这些consumer的后续代码、配置、依赖或环境变化都会使早期诊断
证据失效；最终candidate仍须由J唯一owner建立一次对应的验收结果。
