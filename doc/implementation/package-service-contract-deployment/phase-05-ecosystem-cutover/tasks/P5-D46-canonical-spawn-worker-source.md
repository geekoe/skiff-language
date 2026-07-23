# P5-D46：Canonical Spawn Worker Source Design Gap

状态：设计分支暂停，不阻塞I02的canonical spawn submit typed response。

现有production spawn worker只从legacy `ServiceRuntimeContext`启动；RuntimeAssembly没有spawn-worker/route projection，
因此claim/renew/complete/fail没有合法pinned ActivationIdentity source。F45C已使无context worker发送前fail closed，
F45D对实际收到的structured frame已严格授权。

后续设计必须决定：

- spawn worker是否成为每个ActivationContext/assembly replica的显式owner；
- queue claim如何选择active或draining activation，以及worker lifecycle如何随generation drain；
- projection/artifact是否新增worker route，还是由现有deployment runtime capability导出；
- 相应recoverable identity、lease与重启语义。

不得恢复legacy service/build inference或用ambient runtime connection补identity。本节点需单独更新权威设计后再实现；
当前Phase 5 I02只验证canonical spawn submit typed response，不声称claim执行完成。
