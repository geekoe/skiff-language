# P5-I35：Actor Control / I02 Combined Result

结论：FAIL，仅fixture invocation合同参数缺口。

production candidate `dada6d56a42d5eb917ec96db200fc2567b8195df`上，shared Rust 96/96、Runtime host
25/25、Router 58/58、Router type-check、I02 direct 6/6及diff check全部PASS。normal-source fixture命令在
compile前因缺少CLI必需`--artifact-root`退出；没有compiler/Runtime/Router/fixture production失败。

上述PASS证据继续有效。I35A只用新建hermetic临时artifact root复验fixture compile/test；不得重复其它combined。
