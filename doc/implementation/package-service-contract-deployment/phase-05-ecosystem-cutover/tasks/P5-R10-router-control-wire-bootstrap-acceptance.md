# P5-R10：Router Control-Wire Bootstrap Acceptance

未参与F09实现的独立只读Agent。输入为F09 exact clean commit/tree、D11/F09合同、F03A2 shared-wire PASS与F04A
保留的真实失败证据；不得编辑、提交、修复或给F04/R02 verdict。

必验：

- production server只有一个Runtime endpoint/dispatcher/disconnect owner；缩减版第二endpoint不再拥有生产socket；
- 首帧capabilities与后续activation/control全为binary shared frame，方向、payload与identity fail closed，无text/bare/
  ACK/compat fallback；
- capability session先于registration且身份不可变，health分别显示capability connection与committed healthy replica，
  未admit连接不可dispatch；
- runtime.register/health、request/response/cancel、connection.send、actor/spawn既有接受集未缩窄；
- 未修改Runtime、shared codec/corpus、activation store/gateway、F05、test infrastructure、manifest或lock，未提前实现
  F03C；
- `extra-review`确认没有第二codec、第二session registry或巨型混合dispatcher，并运行F09全部聚焦门禁。

第一行只给`R10 PASS`或`R10 FAIL`。PASS只允许在exact合流状态恢复F04A preserved isolated Host probe；真实Host
证据仍不可由Router单测、Host单测或ready字符串替代。
