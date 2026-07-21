# P5-F13：Runtime Canonical Unary Bridge

## 输入、owner与限制

- 输入：D14完成；与F12相同code/docs base并行，独立worktree/branch，一个clean commit，不merge/push。
- owner只限Runtime Host session canonical request decode、active-route trust bridge、request entry与直接tests；允许拆分现有
  session tests并新增聚焦`assembly_wire`模块。
- 不改shared transport/request codec、Router、loader/admission/lifecycle、native/effects、WS/serverStream、artifact/store、
  test fixture、manifest/lock、F05或stable。

## 完成态

Router session收到`request.start`只用shared strict `RuntimeAssemblyRequestStart` decoder；不得canonical→legacy重试。
只接受D14冻结的gateway/unary/runtimeAssembly/HTTP/test-effect集合与opaque payload。

新assembly wire bridge从内存active route按ingress查找，并在进入执行前复核assembly identity、generation、contract
operation、host/method/path与current committed route；竞态或错tuple fail closed。bridge以route中真实package build、
service/protocol与operation构造internal `RequestEnvelope`，不读artifact、不信任wire legacy字段。取得的`Arc` route在单
请求期间保持pin；完整old-generation drain仍留F03C。

成功/错误继续走现有response.end/error，cancel继续走request supervisor；same socket/requestId ownership与单terminal
不变。不得因为`httpRequest`存在自动选择legacy `binary_http`或强制`std.http.HttpResponse`；package-test zero-arg void
callable必须沿normal unary lane执行。

## 写入与验证

写集限于`runtime/host/src/host/router_session.rs`、`request_entry.rs`、`request_entry/assembly.rs`、新增
`request_entry/assembly_wire.rs`及聚焦session/request tests；不在约4.8k行旧test文件继续堆大fixture。

```bash
cargo test --locked -p skiff-runtime-transport runtime_assembly_request_start
cargo test --locked -p skiff-runtime-host runtime_assembly_request
node scripts/check-runtime-crate-dag.mjs
git diff --check
```

每个filter非零。正例覆盖真实canonical bytes、zero payload、active exact route、void package-test、nested provider call；
负例覆盖legacy/flat/duplicate/unknown、错identity/generation/operation/HTTP、test effects、adapter/stream、route activation
竞态与cancel/terminal ownership。回报decode→route→envelope→response矩阵、commit/tree/lock、single clean、reverse与
extra-review。

## R13 acceptance record

candidate `7cebf4cd85df9ce404ad66ced6bdd3cc8b6683ad` / tree
`040fdbc6cb04dda8e16205cdcd735f5a6251074e`与F12合流为`02b97ff`后由独立R13判定PASS。Runtime strict decode、
active-route exact trust/Arc pin、route-owned envelope、response/cancel通过；真实combined probe完成11个HTTP unary PASS。
