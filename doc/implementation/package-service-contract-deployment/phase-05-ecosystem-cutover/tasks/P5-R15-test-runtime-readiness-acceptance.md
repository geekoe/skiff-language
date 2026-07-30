# P5-R15：Test Runtime Readiness Acceptance

未参与F15实现的独立只读Agent。输入为F15 exact clean commit/tree、D16/F15合同与R14 PASS tree；不得编辑、提交、
修复或给F04/R02 verdict。

必验：

- barrier位于activation 2xx后、唯一业务request前，查询同control origin并strict typed decode；
- pending、active exact tuple、healthy+connected replica与matching connected capability四项同时成立才放行；
- stale只等待，forward/mismatch/malformed/non-2xx立即失败，deadline有界且无固定sleep；
- request exactly once，无503/timeout重试、旧generation fallback或ambient state；
- 未修改Router/Runtime/wire/receipt/fixture/source suite/manifest/lock，无第二readiness owner或大integration fixture膨胀；
- 运行F15全部门禁与`extra-review`，base exceptions不记PASS。

第一行只给`R15 PASS`或`R15 FAIL`。PASS只允许exact合流后由原F04 gate owner在新隔离环境原样跑完整suite；D16临时
barrier、direct tests或健康字符串不能替代最终Host结果。

首次`R15 FAIL`冻结四项复验：DNS/I/O deadline、canonical pending invariant、strict UTF-8与模块职责边界。F15A必须
从原base重建single candidate；正式窄复验还需确认其余已通过state/request-once矩阵未回归，并消费root唯一combined
package-service test证据。
