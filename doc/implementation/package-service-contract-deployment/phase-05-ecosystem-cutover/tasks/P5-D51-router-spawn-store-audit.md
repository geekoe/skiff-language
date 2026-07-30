# P5-D51：Router Spawn Store Audit

DAG节点D51，依赖F50A PASS。冻结production commit
`486379da61beae8f0baf6bf72dfee288e43e2204`；F50A已排除Runtime host continuation。

两个全新只读分片可并行：

- D51A：审计Router从收到`spawn.submit.request`到`ensurePolicy/enqueueSpawn`及typed response write的await链，
  包括Mongo/store schema、index/transaction、错误response和无终止future风险；对照现有direct测试覆盖。
- D51B：审计isolated I02 harness的Router/Mongo/store配置与生命周期，以及I02D为什么未保存router/runtime日志内容；
  判断环境差异、store初始化或日志证据缺口是否能解释20秒timeout。

只返回事实、唯一owner候选、最小非E2E探针、遮挡关系与证据失效面。禁止编辑、提交、重跑I02/R05/instance/
stable/full gate，不作I02/R02 verdict。汇总后若仍不能静态定位，先实施最小Router direct/store probe，不再运行完整I02。
