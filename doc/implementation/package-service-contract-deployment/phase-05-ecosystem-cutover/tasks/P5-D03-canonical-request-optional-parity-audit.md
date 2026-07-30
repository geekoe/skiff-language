# P5-D03：Canonical Request Optional-field Parity Audit

## 角色与输入

由未参与F03A实现的只读Agent执行。输入是R02A首次FAIL的exact candidate
`a7566bb2619ea43f88683ce2f83b4fc4bb441c94`、权威设计、F03A/R02A合同，以及TS
`validateRequestStartFrameHeader`/`validateRuntimeAssemblyRequestRouting`与Rust
`RuntimeAssemblyRequestStartFrameHeader`的production decoder。不得修改、提交或运行consumer/live gate。

## 审计边界与完成态

逐字段列出canonical request允许的全部top-level optional field及其nested closure：presence/nullability、JSON
type、unknown-field policy、enum/pattern、integer/safe-range、empty-string、array item、duplicate与跨字段条件。
对每一项给出TS/Rust当前接受集合、最小counterexample、冻结后的唯一期望以及mutation名称；至少覆盖：

- activation/gateway/business/websocket identities、clientSession、deadline、trace；
- httpRequest、httpAdapter callable/adapterArgs/source；
- websocketAdapter、context expectation、connect/receive/message/payload segments/context codec；
- testEffectsEnabled/testEffectDoubles与canonical caller/routing之外的共同header字段。

审计必须区分wire安全所需的跨语言一致性与Router-only业务语义；不能靠删除F03B/F03C需要的字段、放宽Rust
`deny_unknown_fields`、引入第三套fixture parser或把legacy request改成canonical来闭合。输出一份可直接实现的
字段矩阵、mutation最小完备集与建议owner拆分；不作R02A新verdict。
