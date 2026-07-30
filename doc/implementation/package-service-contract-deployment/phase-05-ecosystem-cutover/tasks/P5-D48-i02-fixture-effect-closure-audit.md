# P5-D48：I02 Fixture Effect Closure Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第5、7、10、11条，§6–§8、§12及§14。

DAG节点D48，依赖I02B exact FAIL。全新只读Agent闭合I02 normal-source fixture及其后续transaction路径，第三次I02前
一次找齐effect/ABI/receipt同类问题：

- unary operation如何独占canonical spawn submit并返回typed receipt；
- WebSocket ingress如何只调用non-suspending pure marker，保持protocol/operation identity与R05证据隔离；
- api.yml/deployment bindings/fixture receipt期望是否需分开operation；
- author/store、activation、unary、withdrawal、tamper/reject/rollback后续是否仍有被compile blocker遮挡的字段/owner；
- 最小fixture/scripts修复、direct compile/test、cheap combined及证据失效面。

只允许`rg`、`git log/show/diff`与源码/测试静态读取；禁止编辑、提交、构建、测试、fixture/I02/R05、instance/stable。
不得放宽compiler ABI、移除spawn typed-response完成标准或用legacy/manual emitter。输出一次批量修复DAG及第三次I02解除条件。
