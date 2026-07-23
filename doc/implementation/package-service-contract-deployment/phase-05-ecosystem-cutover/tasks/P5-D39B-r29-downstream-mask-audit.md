# P5-D39B：R29 Downstream Mask Audit

权威设计为
`doc/architecture/package-service-contract-deployment.md`：

- §5“ServiceDeployment”中Ingress只绑定ContractOperationId、显式operation mapping及完整dependency binding条款；
- §6.2“Service boundary call”中调用必须经过service dispatcher并切换ActivationContext owner的条款；
- §7“Linkable、Recoverable 与 Callback Capability”中callback capability owner/lifetime与runtime capability table条款；
- §12“RuntimeAssembly 与扩容”中单一active assembly、replica加载完整同一assembly以及health/atomic reload可观测条款；
- §14“Fail-closed 条件”中dependency、identity、callback/native adapter与ActivationContext错误不得退化或猜测的条款。

DAG节点D39B，依赖R29在exact candidate
`8982107308c021fe9a72ad9446e1820395a0bc83`于bootstrap identity检查处FAIL。该失败遮挡activation generation 1、
readiness、唯一业务WebSocket、Event/Result materialization、native direct-send marker与cleanup。本节点只审计这些尚未
动态观察的下游生产跳点，不重复D39A identity传播面，不作R29或阶段verdict。

全新只读Agent必须建立从“bootstrap strict identity检查通过后的下一行”到最终cleanup的闭合矩阵，逐跳列出：

- production owner、真实输入/输出schema、exact tuple/identity/owner边界；
- F27C/I27现有正反证据及其是否只来自mock/fixture；
- R29仍未观察或被上游失败遮挡的范围；
- 可在不启动服务时提前执行的便宜探针，以及真正只能由下一次完整真实smoke覆盖的最小范围；
- 独立blocker（如有）的最小写入owner、互斥边界与证据失效面。

只允许`rg`、`git show/diff/log`、源码/fixture/既有测试静态读取；禁止编辑、提交、执行测试、Cargo、Node smoke、
Router/runtime/activation、full/I16/Host/stable。不得把test double当production证据，不得提出业务retry、fake peer或协议
降级。若发现需要改变公共contract、activation/health语义或callback lifetime，明确标记设计决策并暂停受影响分支。

证据仅对上述exact production candidate、tree
`f7457b1d11a43406763184e8ff220277d6ac6049`及Cargo.lock blob
`f3ce5457138c58aec4c84abda431afa96013e3fd`有效；smoke、Router/runtime wire、activation/readiness、eval/native或fixture
变化会使审计失效。交付事实、缺口和便宜探针，不修改代码。
