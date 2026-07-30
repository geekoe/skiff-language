# P5-F51B：Isolated Failure Log Evidence

DAG节点F51B，依赖D51 COMPLETE。独立worktree，唯一写入范围为isolated test runtime失败诊断helper及其Node测试。

在`testError`存在时，停止受管进程后、删除workspace前读取Router/Runtime stdout/stderr日志；为每份记录原始
byte count、SHA-256、truncated标志与经过既有secret redaction/temp-path归一化的bounded tail，并把证据附到
外层可原子写入worktree外ledger的错误对象。之后必须继续执行相同ownership/PID/port/workspace cleanup。

禁止保留整个workspace、输出全日志/secret、吞掉原始错误、改变成功路径或production协议。测试必须覆盖长日志
截断、hash/bytes、secret/path脱敏、缺失/空文件、清理后证据仍可用及cleanup失败组合。运行Node命名测试、
`node --check`、`git diff --check`并提交单一commit。禁止I02/R05/instance/stable/full gate、push/merge。
