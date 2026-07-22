# P5-R16：F04 Production Path Narrow Acceptance

使用未参与F16A/B/C/F17/F18A–I实现、D20审计/修复、I16或窄验收的全新独立只读Agent。输入为D20闭合矩阵与
九个repair ledger、同一exact clean commit/tree上的I16 combined PASS、R15B/compiler trust/Router CAS/resource
lifecycle窄验收PASS及H18 focused-negative PASS；不得编辑、提交、修复、操作stable或运行完整
source-suite/Host，也不作F04/R02总体verdict。权威设计为
`doc/architecture/package-service-contract-deployment.md` §3、§6.1、§6.2、§9–§14及阶段标准2/4/5/6。

第一行只给`R16 PASS`或`R16 FAIL`。必验：

- D20矩阵从真实入口到最终可观察结果的全部14个跳点均有唯一production owner、输入/输出边界、正反probe、exact
  evidence与unseen结论；上游遮挡范围已经检查，独立blocker已批量关闭，无未决公共契约/架构/业务语义问题。
- F18A–I各自exclusive owner无越界；R15B关闭候选Clippy，Router file CAS跨实例竞争与process/gate resource
  no-clobber由独立窄验收关闭；H18真实观察false assertion→Runtime diagnostic→Router non-2xx→runner exit1且请求一次。
- F16A是唯一platform trust owner；library、binary authoring、runner、smoke fixture、source-suite、`skiff test`显式消费
  同一个root；无cwd/env/executable/`CARGO_MANIFEST_DIR` production fallback、第二helper、dual path或clean-cache依赖。
  只允许ignored Rust identity probe读取`SKIFF_TEST_PLATFORM_SOURCE_ROOT`；`__ecosystem-store`无源码action不构造context。
- I16确实以A-built/Fresh-B、不同worktree和共享target关闭identity/structure矩阵；候选、binary hash/mtime、lock与环境
  证据一致，8个identity值等于D18 golden，production artifact/dep-info无worktree platform常量，source registry不扩张。
- F17真实FileHandle/child交错证据PASS，supervisor无fire-and-forget close、立即exit或第二lifecycle owner；D20覆盖的
  bootstrap/activation/readiness/request/eval/service boundary/result/cleanup生产链与冻结设计一致。
- fake reserved package、missing/cross-root、relative/omitted/context mismatch、invalid activation/request/response和资源
  泄漏负例均由窄证据关闭；未修改Router/Runtime/schema/fixture业务语义/manifest/lock，或任何相关修改已经作为D20
  repair显式验收。
- 直接触碰的大文件职责收敛，运行`extra-review`；不把文件行数本身当finding。

只运行风险所需的便宜聚焦抽查，不重复开发者仍有效测试或I16 combined probe。PASS只解除G16；完整Host尚未运行，
不得提前解除F04 receive。回报blocking issues、non-blocking follow-up、命令、动态缺口和残余风险。
