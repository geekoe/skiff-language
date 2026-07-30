# P5-I30：Lifecycle Consumer / Provisioning Cheap Combined

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第4、5、8、9、10条，§5、§6.2、§7及§12–§14。
DAG节点I30依赖F03B/F03C/F30A合流；PASS只解除最终R05，不完成F05/R02/Phase 5。

风险高；全新只读Agent在exact clean candidate只刷新受影响的共享接线：

- F30A显式store path、local/isolated/remote install与fail-closed；
- F03B canonical store startup/snapshot、F23E Router acquire/release/old-generation pin；
- F03C Runtime acquire/release/session cleanup、retired generation回收；
- TS/Rust shared wire保持bit-identical且Cargo.lock不变。

派发前root根据F30A交付填写exact HEAD/tree/Cargo.lock与direct test文件。每条命令至多一次、测试数非零、fail-fast；不重复
F03B/F03C全部focused suites，不启动Router/runtime/instance，不运行R05 transcript、I02/full/I16/Host/stable，不编辑/
修复/提交。最后运行type/check、runtime DAG、diff/status。任一失败I30 FAIL；全部PASS才解除R05。scripts/config、Router
store/generation consumer、Runtime lifecycle、F23E wire、Cargo.lock或命令变化会使证据失效。
