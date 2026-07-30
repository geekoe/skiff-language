# P5-D53：Service Protocol v1 Residual Audit

DAG节点D53，依赖F52A/B/C/D合流到commit
`650bcb6db02622711c72f13ec53bea0322e8fd20`。两个全新只读分片并行：

- D53A：审计Rust production/tests中的`skiff-protocol-v1`，区分错误的ServiceProtocolIdentity consumer/fixture与
  合法runtime frame schema/version。
- D53B：审计Router/Node fixtures/manifests中的v1，定位会进入register/assembly/spawn SPI字段的真实fixture，
  与仅测试legacy reject的负例分开。

返回完整分类清单、互不重叠修复owner、最小命名测试与禁止机械替换面。禁止编辑、提交、I02/R05/instance/
stable/full gate，不作verdict。汇总后一次清理所有错误SPI v1 residual，再运行I52 combined。
