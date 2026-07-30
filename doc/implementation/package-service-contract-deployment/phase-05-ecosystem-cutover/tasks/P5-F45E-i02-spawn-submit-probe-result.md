# P5-F45E：I02 Canonical Spawn Submit Probe Result

结论：COMPLETE。

- task commit：`3c0d9b09fd1b1bdf03bee21b7d0aff3a171b9012`
- integration commit：`dada6d56a42d5eb917ec96db200fc2567b8195df`
- integration tree：`ccd7445a59455fde24f17d71260d473bd208a658`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

新增I02 normal-source fixture使用既有`spawn acceptSubmittedReceipt(...)`；unary只有在Runtime收到并typed decode
Router submit response后返回稳定业务receipt。ledger记录business result、submitted status及
`workerExecutionRequired:false`。direct 6/6及node/diff check PASS，未触D46 worker。

真实fixture compile与跨层consumer接线由I35唯一owner验证。
