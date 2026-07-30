# P5-D39A：R29 Identity / Receipt Propagation Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md`：

- §2“不变量”第9条：code identity、protocol identity、deployment revision与assembly identity必须分开；
- §3“Package 与 PackageArtifact”：PackageArtifact及其identity来自规范compile结果，storage locator不参与语义identity；
- §13“Registry、Release 与 Publish”：typed immutable records与可更新pointer分离；
- §14“Fail-closed 条件”：dependency、version或identity不匹配必须在请求前失败，禁止从raw JSON或display name补事实。

DAG节点D39A，依赖R29在exact candidate
`8982107308c021fe9a72ad9446e1820395a0bc83`的首次且唯一运行FAIL。首错是bootstrap receipt的实际std
PackageBuildId为`2541456b...`，smoke oracle仍期待已被c277e45新增WebSocket公共ABI淘汰的`3bbab8df...`。本节点只返回
identity/receipt传播事实、缺口与便宜探针，供root汇总修复DAG；不作R29或阶段verdict。

全新只读Agent独占审计以下风险面：

- 从`CompilerPlatformSources`、F27A typed publication receipt、F27B seed/bootstrap receipt到F27C smoke oracle的
  build/prelude identity来源与数据流；
- 当前repo内旧/新build与prelude常量的所有live consumer，区分production expectation、test regression pin、历史
  gate ledger和纯文档证据；
- receipt是否已经携带足够typed facts使oracle无需第二个手写identity事实源；
- 每个独立缺口的production owner、最小写入边界、正反便宜探针，以及哪些既有证据会失效。

只允许`rg`、`git show/diff/log`、源码/fixture静态读取及不会启动服务的schema解析；禁止编辑、提交、测试、Cargo、
Node smoke、Router/runtime/activation、full/I16/Host/stable。不得把常量替换方案当作既定结论；若现有production receipt
不足以表达权威identity，必须标记为设计/契约缺口并暂停该分支。

证据仅对上述exact production candidate、tree
`f7457b1d11a43406763184e8ff220277d6ac6049`及Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`有效；identity算法、platform source、F27A/B/C或receipt schema变化会使审计
失效。交付闭合矩阵与事实，不修改代码。
