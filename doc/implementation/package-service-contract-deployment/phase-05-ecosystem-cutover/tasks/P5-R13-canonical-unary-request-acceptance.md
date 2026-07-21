# P5-R13：Canonical Unary Request Acceptance

未参与F12/F13实现的独立只读Agent。输入为两个exact clean commit/tree、D14/F12/F13合同与F03A2 PASS shared seam；
先在integration合流两端，再做combined receive。不得编辑、提交、修复或给F04/R02 verdict。

必验：

- Router只产生shared validator接受的canonical nested unary header，零payload/opaque bytes、HTTP cross-check、cancel与
  terminal ownership正确；无legacy/flat/build rewrite/JSON强标；
- Runtime只用shared canonical decoder，active route exact复核并pin，internal envelope只取可信route facts；无fallback、
  artifact read、current-pointer-only dispatch或binary_http误推断；
- 双端对schema/raw/optional/default/unknown/duplicate/unsafe generation接受集合一致，requestId/socket/terminal/cancel
  ownership不漂移；
- package-test capability仍false，normal HTTP ingress可执行zero-arg void wrapper与nested provider call；不恢复legacy
  package-test seam；
- WS/serverStream/httpAdapter/test doubles/drain保持未实现且fail closed，shared codec/corpus、store/activation、manifest/
  lock不变；
- 运行F12/F13全部门禁、F03A2 cross-language corpus与真实Router→Runtime unary component probe；`extra-review`确认无第二
  writer/decoder/dispatcher/pending map或巨型混合职责。

第一行只给`R13 PASS`或`R13 FAIL`。PASS只允许exact合流后恢复F04真实Host suite；组件正例、std assembly或日志字符串
不能替代checked-in consumer最终Host结果。
