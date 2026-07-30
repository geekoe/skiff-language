# P5-F446D Test Runner And Ecosystem Migration

## Scope

Skiff test-runner：

- 一个test service execution仍只构造一个multi-root RuntimeAssembly和一次activation generation；
- 每个generated deployment在同一RuntimeConfigSnapshot中有独立分区；
- runner把动态普通HTTP `skiff.test.ingressUrl`作为对应Package的只读runner overlay写入分区，authored同名
  path失败；
- 每个case DB由`(testRunId, generatedTestServiceId)`派生并清理，不从profile读取namespace；
- 不提供per-case authored config override；需要不同author config继续使用另一个test service。

生态迁移：

- Skiff fixtures、official packages、Internals services/tests把三层文件改成Package-ID root mapping；
- 删除全部manifest/profile `state`、SecretRef declaration、timeout/quota/principal/resources占位字段；
  `package.yml.resources`静态资源不在删除范围；
- secret明文文件内容不得输出或提交，只迁移key层级并保持ignored/`0600`；
- stable dev/watch使用snapshot构造链，不复制旧deployment config binding；
- 删除旧fixture/helper/checker和无消费者的兼容代码。

跨仓库分别提交和合并；不得记录submodule指针，不push。

## Evidence

聚焦test-runner isolation/self-ingress/DB tests、各仓库authoring checker、official与Internals全non-live、
stable activation cold restart和Agine chat smoke。任何secret只报告path presence与权限，不报告值。
