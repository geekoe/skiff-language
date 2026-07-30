# P5-F398 Builtin canonical spelling audit

状态：Ready（只读）。

## 直接父节点

- `P5-F397-test-runner-http-gateway-final-retry-blocker.md`

本节点确定`boolean`/`bool`的唯一canonical owner及同类builtin alias影响面；不通过放宽Runtime linker
比较来掩盖producer漂移。

## 必须完成

1. 从语言reference、parser/type resolution的builtin表确认每个source spelling与canonical builtin name，
   特别是`boolean`/`bool`；区分合法source alias和artifact canonical wire spelling。
2. 逐跳追踪`std.db.ConflictError.retryable`：
   source → resolved type → FileIR → linked type → PackageSchema → linker validation，确定首次分叉。
3. 全量扫描fresh std及代表性package artifact中所有builtin名称，列出可能同类pair，不限于boolean。
4. 判定唯一修复：
   - compiler/FileIR producer统一canonical spelling；
   - PackageSchema producer错误；
   - 或两边应共享artifact-model canonical enum/helper。
   Runtime linker仍应exact compare canonical fact，不增加alias/dual-read。
5. 列出identity影响、需要重建的artifact DAG、最小production/test owner与focused命令。

## 边界与交付

Skiff/std/其它仓库production只读；可生成temporary fresh artifact、跑现有测试，不修改源码，不访问
stable/live，不派子Agent。

在本任务worktree写`P5-F398-builtin-canonical-spelling-audit-result.md`。现有规则若确定唯一修复，返回
`TASK_EXECUTABLE`；只有规范本身冲突才提出用户决策。

result本地commit、worktree clean；不merge/rebase/push。
