# P5-F426A WebSocket connect current wire checkpoint

状态：Ready。高风险跨语言wire checkpoint。

## 直接父节点

- `P5-F426-connect-wire-and-http-consumer-wave.md`

owner细节见父节点引用的F424A audit与F425A result；不得恢复receive/context/message wire。

## DAG位置

依赖F425A，完成后解除Runtime/Host与Router consumer并行节点。本leaf只冻结current
`websocketConnect` request/response wire，不执行connect handler、不启动gateway。

## 写入范围

仅允许：

- `router/src/protocol/runtimeAssemblyRequest*.ts`
- `router/src/protocol/envelope.ts`
- `router/src/protocol/runtimeProtocol.ts`
- 上述wire的Router直接tests/cross-system corpus
- `runtime/transport/src/runtime_assembly_request.rs`及其
  `runtime_assembly_request/{metadata,lexical,strict_json,tests}.rs`
- connect result所需的runtime transport response metadata/wire及直接tests
- language-neutral request/response JSON golden corpus与parity tests
- 本leaf result

禁止修改artifact/compiler/deployment authoring、runtime activation/request/eval/host执行、
Router gateway/dispatcher/server、test-runner fixtures、Internals或skiff-packages。

## 必须实现

1. current request wire是HTTP与`websocketConnect`的closed discriminated union；HTTP现有canonical bytes
   保持不变。
2. connect header精确携带routing、connectionId、URL、query、headers、cookies、可选version、
   canonical `websocketEntryId`和`gatewayEntryIdentity`。
3. connect request没有body业务schema、receive/message、Context、context codec、operation id或
   `ContractOperationId`。
4. response wire精确表达accept/reject：
   - accept只有可选business identity和可选connection policy；
   - reject只有code/reason；
   - 没有payload context、context presence或message response。
5. TS/Rust strict readers拒绝unknown field、wrong discriminator、missing identity、legacy receive/context
   shape、HTTP/WS metadata混搭和noncanonical ID。
6. request/response canonical JSON在TS/Rust exact parity；新增language-neutral golden，不能各自产生hash
   或默认事实。
7. 无handler synthesized accept不通过runtime wire；该行为留给Router consumer。

## 验证

运行真实匹配的聚焦suite并记录discovery：

```bash
cargo test -p skiff-runtime-transport runtime_assembly_request
pnpm --dir router test -- runtime-assembly-request
pnpm --dir router exec tsc --noEmit
cargo fmt --all -- --check
git diff --check
```

若wire实现要求修改runtime/Router execution owner，返回`TASK_SCOPE_EXPANDED`。

## 交付

提交implementation，再新增并提交
`P5-F426A-websocket-connect-current-wire-result.md`。返回commit/tree、parity vectors、自验收矩阵和clean
状态。不得merge/rebase/push/stable/live。

