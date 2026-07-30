# P5-R12：Std Exact Callable Effects Acceptance

未参与F11实现的独立只读Agent。输入为F11 exact clean commit/tree、D13/F11合同、F06/R06与F07/R07已通过边界、
完整std失败证据；不得编辑、提交、修复或给F04/R02 verdict。

必验：

- 10个native key与3个receiver identity逐项exact，语义/context/handler route一致；四个既有string项不漂移；
- source resolved facts是唯一target identity owner，lowering不再独立推导receiver；runtime只验证实际可达handler，不建
  第二semantics registry；
- crypto/date/duration/number/receiver pure scalar，sleep只有may_suspend；Time context不同route不得blanket合并；
- 完整std恰好11 cases全部Available/assemble，helper mutation仍Unavailable而consumer/test仍Available；
- unknown/dynamic/first-class/mutable receiver与file/http/websocket capability natives、forged projection继续fail closed；
- 无std/runner/boundary/schema/identity/deployment/Router/fixture/manifest/lock越界，所有filter非零并运行F11全部门禁；
- `extra-review`检查无第二target parser、第二effects owner、重复registry或把新语义堆入巨型mixed-responsibility文件。

第一行只给`R12 PASS`或`R12 FAIL`。PASS只允许exact合流后恢复F04真实Host probe；std assembly、runtime handler单测或
源码字符串不能替代后续checked-in consumer Host结果。
