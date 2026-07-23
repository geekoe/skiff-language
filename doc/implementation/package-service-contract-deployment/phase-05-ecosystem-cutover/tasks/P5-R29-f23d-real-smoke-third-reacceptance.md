# P5-R29：F23D Real Smoke Third Reacceptance

未参与F23/F24/F25/F27及旧R26/R28的全新只读Agent。依赖I27 PASS；同一exact clean candidate只运行一次真实isolated
Router+Rust runtime smoke，不重跑combined/full/I16/Host/stable，不编辑/提交。

必须观察normal source→canonical std store closure→compiler/deployment/assembly→strict receipt→activation generation1→exact
readiness→single WS connect/receive→Event/Result materialization→native direct-send marker，cleanup完整；禁止fake/protocol peer/
业务retry。第一行只给`R29 PASS`或`R29 FAIL`；PASS完成F23D并解锁R24，FAIL保留F26A bounded diagnostic且不得重试。
