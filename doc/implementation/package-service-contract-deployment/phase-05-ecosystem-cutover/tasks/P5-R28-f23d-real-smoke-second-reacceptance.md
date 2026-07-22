# P5-R28：F23D Real Smoke Second Reacceptance

未参与F23D/F24/F25及旧R26的全新只读Agent。依赖I25 PASS；同一exact clean candidate只运行一次F23D真实isolated
Router+Rust runtime smoke，不补跑combined/full/I16/Host/stable，不编辑/提交。

必须从normal source经compiler object Construct、deployment/activation、production Router、Rust boundary/eval/native direct-send
到client marker；可观察Event参数、ConnectResult返回、receive null和cleanup。禁止fixture手补、protocol peer、fake/manual emitter。
第一行只给`R28 PASS`或`R28 FAIL`；PASS完成F23D并解锁R24，FAIL给第一错误且不得重试。
