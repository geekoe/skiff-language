# P5-D87：Codex Relay 首个 Unknown Call 审计结果

结论：`READY_TO_IMPLEMENT`

## 父节点

- `P5-D87-codex-relay-first-unknown-call-audit.md`

## 首个污染点

F152后unsupported boundary已消失。`adminSession`链首个污染是精确native targets：

- `std.http.request.headers`
- `std.http.request.cookie`

两者有native signature并被resolved target精确识别，但`STD_NATIVE_CALLABLE_SEMANTICS`无entry，
`apply_exact_native_call`因此fail closed为unknown，随后沿local summary传播。artifact省略local/native target不是unknown原因。

先只补这两个exact semantics；其他std.http native、动态any receiver、未登记native继续Unknown。随后真实source transfer探针
与新一轮Codex Relay审计再定位下一污染点。

