# P5-I35B：Spawn Submit Fixture Final Reacceptance

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第10、11条，§3、§7、§12–§14。

DAG节点I35B，依赖D47 COMPLETE，是同一fixture路径熔断审计后允许的第三次且最后一次复验。exact production
candidate保持`dada6d56a42d5eb917ec96db200fc2567b8195df`、tree
`ccd7445a59455fde24f17d71260d473bd208a658`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

全新只读Agent必须逐字执行
`P5-D47-i35-fixture-artifact-provisioning-audit-result.md`所引用的D47 final ledger中“第三次唯一fixture owner”完整
`node --input-type=module`代码块一次。该命令：

- 创建并断言空hermetic source artifact root；
- 用`bootstrapCanonicalArgs --bootstrap-only`发布canonical official std；
- 用`skiff test --deny-skips --require-tests`编译/执行I02 spawn-submit fixture；
- 10分钟deadline、signal转交、owned-process 30秒cleanup并删除root复核。

随后只运行一次`git diff --check`。禁止重复I35其它证据、编辑、提交、真实R05/I02、instance/stable或完整gate。
第一行`I35B PASS`或`I35B FAIL`；FAIL给精确bootstrap/compiler/test owner且停止。PASS与I35其余证据共同关闭I35并解除
R05C。
