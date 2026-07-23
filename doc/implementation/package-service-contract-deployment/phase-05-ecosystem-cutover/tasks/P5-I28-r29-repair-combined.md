# P5-I28：R29 Repair Cheap Combined

权威设计为
`doc/architecture/package-service-contract-deployment.md` §2第8、9条、§3、§12–§14。DAG节点I28依赖F28A/B/C全部合流；
PASS只解除R30，不完成F23D。风险高；全新只读Agent在exact clean candidate执行，每条命令至多一次，fail-fast，不编辑/修复/
提交。

组合覆盖：

1. F28C current prelude regression tests；
2. F28A专用Node正反oracle tests；
3. 唯一actual `--bootstrap-only --locked` Rust receipt→production JS oracle无服务交接test；
4. F28B永不activation/open/close与outer cleanup专用Node tests；
5. 受影响JS syntax、`git diff --check`与tracked clean。

验收Agent派发前由root把合流后的exact HEAD/tree/Cargo.lock及精确测试命令补入派发信息；不得另跑F27A/B全套、Router/runtime、
真实smoke、full/I16/Host/stable。测试数必须非零。任何失败直接I28 FAIL，不重跑/修复；全部PASS才解除R30。compiler source、
platform source、bootstrap/oracle/smoke lifecycle、Cargo.lock或命令变化会使证据失效。
