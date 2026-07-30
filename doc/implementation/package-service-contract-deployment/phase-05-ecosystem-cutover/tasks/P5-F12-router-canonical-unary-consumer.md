# P5-F12：Router Canonical Unary Consumer

## 输入、owner与限制

- 输入：D14完成；exact code integration `2d74b2cd084a474667850372b334d223e41c7521` / tree
  `5a497262f1a2cf29f81b4e12a958edc2a729cbe1`，已包含F11/R12 PASS checkpoint。
- 与F13从同一docs base并行；独立worktree/branch，一个clean commit，不merge/push。
- owner只限Router assembly HTTP unary builder、assembly registry/dispatcher/endpoint的canonical header透传与直接tests。
  generic runtime registry只可增加显式fail-closed类型guard。
- 不改shared protocol/codec/corpus、Runtime、WS gateway、serverStream、activation/store、test-runner/fixture、manifest/lock、
  F05或stable。

## 完成态

`AssemblyHttpGateway`用独立canonical builder产生D14冻结的nested header；删除`target/operationAbiId/buildId/
serviceProtocolIdentity`与flat assembly字段。builder须调用shared validator或产生其exact typed输入，不复制parser。
HTTP request body作为opaque bytes传递，零字节保持零字节。

Assembly registry与dispatcher只把canonical header/payload原样交给统一Runtime endpoint，不做build-id rewrite或JSON解析；
无`httpResponse`时不得把RuntimeBinary强标JSON。timeout/caller abort仍发送同requestId空payload cancel，unary只接受单
terminal end/error；response start/chunk fail closed。WS对旧builder的调用保持原状，等待F05。

## 写入与验证

写集限于`assemblyHttpGateway.ts`、`assemblyRuntimeRegistry.ts`、`runtimeDispatcher.ts`、`runtimeEndpoint.ts`、
`runtimeRegistry.ts`的窄guard，以及现有assembly tests与新增`runtime-assembly-unary-dispatch.test.ts`。

```bash
pnpm --filter @skiff/router type-check
pnpm --filter @skiff/router test -- \
  tests/host-ingress.test.ts \
  tests/assembly-replica-dispatch.test.ts \
  tests/assembly-runtime-endpoint.test.ts \
  tests/runtime-assembly-unary-dispatch.test.ts
git diff --check
```

真实socket test必须用shared validator独立核验nested bytes，覆盖零payload、HTTP交叉校验、legacy/flat/unknown、stream/
adapter、response/cancel ownership。回报header矩阵、commit/tree/lock、single clean、scope/reverse与extra-review；不得用
mock registry直接调用替代writer→dispatcher→socket证据。

## R13 acceptance record

candidate `7966491e6b7c7850cb76e5b9291b848f1fed4e9e` / tree
`ea024d177110e20c3d91b37a5abe7e903b7469af`与F13合流后由独立R13判定PASS。Router nested writer、opaque/zero
payload、HTTP cross-check、terminal/cancel ownership与真实socket通过；shared codec/corpus/lock未变。
