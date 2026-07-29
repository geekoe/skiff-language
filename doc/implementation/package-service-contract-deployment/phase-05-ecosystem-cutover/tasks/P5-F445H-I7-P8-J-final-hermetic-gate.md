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
4. 受P8 production写集影响的Skiff component selector；若R为NO-OP且H/K聚焦证据仍有效，不重复无关
   full workspace。

AIHub可使用用户已授权的临时managed Mongo，只能动态端口、临时目录、sanitized env并在结束后清理。
禁止stable instance、外网、OAuth、browser、真实API key和`defaultRun false` live test。任何skip、零发现、
遗留进程/端口或未清理临时状态均不是PASS。

结果ledger采用：

```text
层级 | 命令 | owner | commit/代码状态 | 结果 | 覆盖范围
```

J不修改候选。失败先分类并退回预验收，不在gate状态顺手修复或连续重跑完整矩阵。
