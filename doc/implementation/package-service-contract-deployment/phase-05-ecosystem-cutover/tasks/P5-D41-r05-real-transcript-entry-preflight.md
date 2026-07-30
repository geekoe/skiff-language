# P5-D41：R05 Real Transcript Entry Preflight

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7、§12及§14。

DAG节点D41，依赖I30 PASS。当前静态预读没有找到可执行的真实
`A connect → activate B → A receive×2 → B connect/receive → unary B → close/release/drain`入口；已有
`run-package-service-ecosystem-smoke.mjs`只覆盖单次generation。D41只读冻结R05真实入口或缺失harness的最小实现合同，不作
R05/R02/Phase verdict。

全新只读Agent在交接文档记录的exact candidate建立闭合矩阵：

- existing isolated runtime owner能否在同一run中author/store assembly A与B并两次activate；
- normal source/fixture如何产生可区分A/B marker且不patch/re-sign artifact；
- client A连接、两次A receive、client B连接/receive、unary B、close/release/drain的production可观察字段；
- Router/Runtime health或diagnostic能否证明旧A retire只发生在最后pin释放后；
- F30A compiler sidecar在isolated dev-home中的真实build/install来源；
- cleanup、deadline、端口/lease/workspace ownership及F26A diagnostic是否可复用；
- 若入口已存在，给唯一精确命令；若不存在，冻结一个新scripts/test-infrastructure开发节点的最小写入边界、直接tests、
  cheap combined与证据失效面。

只允许`rg`、`git show/diff/log`、源码/fixture/既有测试静态读取；禁止编辑、提交、构建、测试、启动Router/runtime/instance、
运行旧smoke或操作stable。不得把组件fake、protocol peer、manual emitter、artifact patch/re-sign或业务retry当作R05入口。
若必须改变公共ABI、activation/release语义或四对象，标记设计决策；否则只给implementation owner。

证据锚定交接文档的production commit/tree/Cargo.lock；scripts/isolated runtime、fixture/authoring、Router/Runtime lifecycle或
provisioning变化会使审计失效。
