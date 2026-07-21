# P5-D13：Std Exact Callable Effects Audit

## 角色与结论

F04B让真实suite进入完整std root后，`crypto.test.skiff`首个case因
`[UnknownCallTarget, RequiresSameHeapIdentity]`被canonical boundary拒绝。D13只读追踪source、overlay、effects、
projection、lowering与runtime registry；不得编辑、提交、修复或给F04 verdict。

结论为`DESIGN GO`：resolver已精确解析`std.crypto.hmacSha1Base64`，但F07冻结的稀疏native semantics只覆盖四个
string builtin，crypto/time/date/duration/number缺descriptor；Date/Duration receiver又由source target builder与
lowering分别推导，形成第二target owner。boundary按架构正确fail closed，runner/fixture无错。

完整std 11 cases中，crypto 0–2与time/date/duration 8–10不可用，string truncate 3–7可用。冻结F11/R12一次扩展
exact production semantics与resolved target facts；不得修改std source/registry、runner policy、boundary eligibility、
artifact identity或Host fixture，不得全局允许unknown/same-heap或创建test hook。

## 冻结对象

新增10个exact native keys：`core.date.now`、`core.duration.milliseconds`、`core.duration.seconds`、
`core.number.assertSafeInteger`、`std.crypto.hmacSha1Base64`、`std.crypto.sha256`、`std.crypto.randomToken`、
`std.crypto.uuid`、`std.crypto.uuidSimple`、`std.time.sleep`。

新增3个canonical receiver target identities：`receiver:Date.isBefore@1`、
`receiver:Date.toEpochMilliseconds@1`、`receiver:Duration.toMilliseconds@1`；分别映射signature key
`core.date.isBefore`、`core.date.toEpochMilliseconds`、`core.duration.toMilliseconds`。receiver identity不得降格为
普通native string。

crypto/date/duration/number与三个receiver op均无caller mutation/alias/escape/same-heap/unknown/suspend；sleep仅
`may_suspend=true`。`core.date.now`与`std.time.sleep`都需Time context但实际route不同，runtime必须逐key核对。未知、
dynamic/first-class、mutable receiver及file/http/websocket capability natives继续fail closed。
