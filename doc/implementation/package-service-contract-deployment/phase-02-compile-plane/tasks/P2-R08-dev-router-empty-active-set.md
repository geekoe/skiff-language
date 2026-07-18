# P2-R08：Dev Router Empty Active Set

状态：deferred；不是 Phase 02 需求或验收依赖。

是否允许空 `RuntimeAssembly`、以及 dev router/runtime 如何表示空 active set，应在 Phase 03/05
基于终态 assembly 与 reload 协议决定。Phase 02 不修改旧 service manifest loader，也不为了测试
启动条件保留 router 特例。

旧 integration 中的本任务代码不进入新 branch。本文件仅记录 disposition，不是可执行指令。
