# P5-D55：Terminal Legacy Consumer Audit

依赖T06 TASK_NOT_EXECUTABLE。三个全新只读分片并行：

- D55A：runtime loader/linker对PackageUnit/ServiceUnit/PublicationAbiUnit的production读写、validation、link输入。
- D55B：runtime host/driver对legacy closure/cache/program/HTTP boundary DTO的production消费与canonical替代输入。
- D55C：linked-program/public exports、artifact-model/identity owner、fixture/checker/doc删除顺序与反搜registry。

每片列完整文件/调用链、语义是否仍可达、canonical replacement、互斥写入owner、最小编译/正负探针和证据失效面。
禁止编辑、提交、I02/R05/full gate。汇总后先落短consumer checkpoint，再恢复terminal deletion；不得shim/alias/fallback。
