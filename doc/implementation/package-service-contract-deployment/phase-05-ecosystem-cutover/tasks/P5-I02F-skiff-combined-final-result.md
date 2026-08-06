# P5-I02F：Skiff Consumer Combined Final Result

> 已由 M4 取代（2026-08-06，runtime-lazy-deploy）：本文所述 activation 协调层 / epoch 部署机制
> （`assembly.activation` 帧族、`/__skiff/activate-assembly`、activation 状态仓库与配置项、
> committed/expected generation 术语等）已在 M4 全部下线，部署语义以
> [`doc/architecture/runtime-lazy-load-deployment.md`](../../../../../architecture/runtime-lazy-load-deployment.md)
> 为准。本文保留为历史执行记录。

结论：PASS。冻结docs HEAD `a4f8e7098e258982e3933287e1439a6e2daba4da`、production commit
`ee21b85ddd70c63585af6961ce4ea1ef8d4ec37e`、tree
`e67a9f23f43b23a26b1915230fa592935f55b7d2`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`精确匹配；smoke与diff各唯一执行一次并PASS。

Ledger：

- generation 1、assembly
  `skiff-runtime-assembly-v1:sha256:9bef6ffb6306b72c3e9613506567d01b97b87d6514575e0431d0774d35985fdb`；
- typed unary result `P5-F45E-SPAWN-SUBMIT-TYPED-RESPONSE:submitted`；
- spawn response `submitted`，source为normal fixture，`workerExecutionRequired:false`；
- request-time artifact I/O为0；两次artifact-root withdrawal后旧unary均恢复且I/O仍为0；
- generation 2 tampered transitive `skiff.run/std` candidate在load阶段reject，未prepare/allocate；
- rollback后committed generation仍为1、assembly不变、旧typed result仍可用、replica connected、
  `pendingActivation:null`；
- artifact root与tampered record均恢复，cleanup complete。

隔离workspace、Cargo target、相关PID及动态端口全部清理，工作树clean。解除R02。
