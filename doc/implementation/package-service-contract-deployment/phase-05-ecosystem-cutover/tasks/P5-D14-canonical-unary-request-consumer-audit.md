# P5-D14：Canonical Unary Request Consumer Audit

## 角色与结论

R12 PASS后真实std generation-1已prepare/commit，但首个请求在native invocation前因Runtime解码
`unknown field assemblyIdentity`断连。D14只读审计Router HTTP gateway→binary frame→Runtime session→Host/eval→
response/cancel；不得编辑、提交、修复或给F04 verdict。

结论为`DESIGN GO`：F03A2冻结的TS/Rust canonical nested request codec正确且bit-identical，production Router仍发送
legacy+flat hybrid header，Runtime仍用旧`RequestStartFrameHeader`。flat被canonical validator与Runtime拒绝，nested又被
legacy decoder拒绝，当前没有共同接受shape。这是F03B/F03C consumer被排在F04后的DAG环，不是F11/native/fixture/
generation问题。

冻结并行F12 Router与F13 Runtime，合流后R13。只接通normal HTTP unary lane：strict nested runtimeAssembly routing、
gateway caller、opaque payload、现有response/end/error/cancel。不得回改shared codec/corpus、恢复flat/legacy fallback、
实现WS/serverStream/httpAdapter/test doubles/drain或读取artifact。

## 冻结接受集

- `request.start`、`mode=unary`、`caller.kind=gateway`、`routing.kind=runtimeAssembly`；
- routing携带exact assembly identity/generation/contract operation/HTTP ingress；`httpRequest`必需且method/path/URL host
  与routing交叉一致；
- `testEffectsEnabled=false`、`testEffectDoubles={}`；payload保持独立opaque bytes；
- legacy top-level、flat assembly字段、unknown/duplicate字段、unsafe generation、http/websocket adapter与stream拒绝；
- Runtime用内存active route复核exact tuple并取得单请求`Arc` pin，再以route中真实package build/service/protocol构造
  internal envelope；不得信任wire伪造legacy target字段；
- unary只接受单terminal response.end/error；cancel复用现有supervisor并保持socket/request ownership。
