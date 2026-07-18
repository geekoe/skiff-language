# P2-R10B：Driver Test Fixtures

状态：cancelled；不再单独执行。

该任务曾迁移旧 service publication driver tests，但其 production 入口在 terminal-only 方案中整体删除。
仍属于 package compile 的诊断由 P2-R10 使用 canonical package fixture 保留；service publication/binding
测试随旧语义删除，不建立 replacement adapter。T05 负责 driver test disposition，T07 负责反向搜索归零。
