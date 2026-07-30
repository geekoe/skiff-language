# P5-R26：F23D Real Smoke Reacceptance

未参与F23D/F24实现及旧smoke失败的全新只读Agent。依赖I24 PASS；在同一exact clean candidate只运行一次F23D合同的
真实isolated Router+Rust runtime smoke，不重跑combined、full/I16/Host/stable，不编辑/提交。

必须从checked-in normal source经compiler/deployment/activation、production registry/dispatcher、Rust boundary/eval/native
direct-send到client marker；禁止protocol peer/fake registry/fake dispatcher/手工emitter。Context=null正例及Event/Result
参数/返回materialization都可观察，cleanup完整。第一行只给`R26 PASS`或`R26 FAIL`；PASS完成F23D并解锁R24。
