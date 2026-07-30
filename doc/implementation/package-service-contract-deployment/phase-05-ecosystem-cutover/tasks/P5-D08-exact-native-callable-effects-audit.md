# P5-D08：Exact Native Callable Effects Audit

## 角色与输入

由未参与F04/F06实现的只读Agent审计canonical std package test为何在调用
`std.string.truncateUtf8Bytes`后不能跨test boundary。输入为native signature/prelude/FileIR/runtime registry、
callable-effects target/transfer及F04真实fail-close case。不得编辑、提交、扩F06或调整F04 gate。

## 审计结论

`artifact-model/src/native_signature.rs::STD_NATIVE_SIGNATURES`是compiler/runtime共享的exact binding identity owner，
但仓库没有可信native effects、`may_suspend`或return provenance descriptor；`NativeRequiredContext`只描述能力上下文，
不能推出副作用。native目前以LocalFunction definition全effects + Unknown(Native) fail-close。

D08冻结F07新增缺省缺席=Unknown的稀疏shared callable-semantics registry。首批只允许逐项审计过、
`RequiredContext::None`的四个标量string native：`isAsciiDigits`、`truncateUtf8Bytes`、
`encodeQueryComponent`、`encodePath`。其它native、custom/unknown、crypto及所有capability native继续fail closed。
审计完成只解锁F07，不解锁F04。
