# P5-R24：F05 WebSocket ABI / Owner Checkpoint

权威设计为
`doc/architecture/package-service-contract-deployment.md`：

- §2“不变量”第4、5、8、9、10条；
- §5“ServiceDeployment”中Ingress只绑定ContractOperationId、显式operation mapping与完整dependency binding条款；
- §6.2“Service boundary call”中service dispatcher及ActivationContext owner切换条款；
- §7“Linkable、Recoverable 与 Callback Capability”中capability owner/lifetime与runtime capability table条款；
- §12“RuntimeAssembly 与扩容”中完整同一assembly、独立activation owner及health/drain/atomic reload可观测条款；
- §14“Fail-closed 条件”中identity、dependency、callback/native adapter与ActivationContext错误不得猜测或降级的条款。

DAG节点R24，输入为R25、R27、I28及R30 PASS后的exact clean candidate；PASS只证明F05 ABI/owner/materialization
checkpoint并解锁F23E，F03B/F03C仍须等待F23E shared wire。风险高，验收分组为F05 WebSocket ABI/owner checkpoint。
当前production candidate为commit `cfeba9dd3f1be97d876847ae6aa9bd40cab79181`、tree
`6fdb93168ba30e5d2074ff0bc0eb96e0b939610c`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`；随后只允许本合同/result文档提交，派发时给exact HEAD。

必须使用未参与F23–F28及R26/R28/R29/R30的全新只读Agent。不得编辑、修复、提交、运行real smoke/full/I16/Host/stable，
也不得给R05/R02/Phase verdict。先复核R25 canonical shape owner、R27 target-object materialization、I28 Rust→JS receipt/
lifecycle及R30真实marker证据在当前production代码上仍有效，再检查：

- 正常source→typed ABI→wire→Runtime boundary→production Router marker保持唯一链路，四对象schema与frozen ABI不变；
- registry、dispatcher、response projector及connection lifecycle各有唯一production owner；
- Cookie/URL/repeated metadata、zero-byte Context、response mutations、identity/sender/direct-send错误、receive
  serialization/backpressure/close/shutdown正反闭合；
- Assembly production tests不得注入fake registry/dispatcher，legacy/manual emitter/protocol-peer不得进入真实证据；
- 对F23–F28触及的同一scope执行`extra-review`补充审查：检查重复dispatcher/projector、跨层知识、长函数/文件中的混合职责与
  测试fixture边界；大小本身不构成finding。

只允许静态源码/测试/commit证据检查、必要的`rg`/`git show/diff`、`git diff --check`与`git status --short`。已有动态证据
有效时不得重复其测试；若静态检查无法闭合某项，报告dynamic gap而不是临时扩大gate。第一行仅`R24 PASS`或`R24 FAIL`，
findings按严重度优先，其后给open questions、证据映射、non-blocking extra-review follow-up与残余风险。FAIL不得修复；PASS
不证明A/B generation lifecycle，也不改判R05 FAIL。相关ABI/schema/Router/runtime/compiler/smoke代码变化会使证据失效。
