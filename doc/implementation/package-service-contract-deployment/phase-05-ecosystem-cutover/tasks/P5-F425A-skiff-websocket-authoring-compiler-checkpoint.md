# P5-F425A Skiff WebSocket authoring/artifact/compiler checkpoint

状态：Ready。高风险共享实现检查点。

## 直接父节点

- `P5-F425-downlink-websocket-implementation-checkpoint.md`

只从本文和直接父节点启动；需要owner细节时读取父节点引用的F424A result。不得重新设计WebSocket。

## DAG位置与输入

这是第一波Skiff共享节点。精确production输入为父节点记录的Skiff commit。它完成后解除current connect
wire节点；Router/runtime consumer仍被wire节点阻塞。

## 写入范围

仅允许：

- `packages/std/{websocket.skiff,api.yml}`及其直接tests；
- `artifact-model/src/{ecosystem_authoring.rs,gateway.rs,deployment.rs,service_unit.rs,websocket_ingress.rs,lib.rs}`及对应tests；
- `artifact-identity/src/{gateway.rs,deployment.rs,runtime_assembly.rs,contract/normalization.rs,lib.rs,tests/**}`；
- `compiler/input/src/service_config.rs`及tests；
- `compiler/driver/generated_deployment.rs`、HTTP projector中为共享exact callable resolver所需的最小抽取、
  新connect projector及compiler tests；
- compiler中legacy WebSocket type projection owner；
- `deployment/**` projection/assembly validation及tests；
- 因closed enum/optional handler产生的编译错误而必须机械更新的
  `runtime/{loader,linker,host}` exhaustive matches，但不得在本leaf实现connect执行。
- `runtime/eval/src/runtime_http_gateway.rs`中因新增closed adapter kind必须更新的穷尽match及其直接
  聚焦test；这里只能让HTTP owner对`websocketConnect`显式fail closed，不得实现connect执行。

禁止修改Router protocol/gateway、runtime request/eval/native业务执行、test-runner fixtures、
Internals或skiff-packages。

## 必须实现

1. `ServiceManifestAuthoring.websocket`改为严格的可选singleton DTO，精确接受父节点YAML shape。
2. `host`默认`"*"`，`path`必填；`connect`可省略。拒绝map/list、多entry、author id、`routes`、
   operation、receive/message/context、unknown/duplicate field。
3. connect target只接受当前package private non-generic callable；adapter source只允许
   `websocket.connectRequest`和`websocket.connectionId`，参数名唯一且与signature精确匹配。
4. std source只保留connect request、policy、non-generic无Context connect result与四个send native/JSON
   helper；删除message/receive/connection/ingress union和user close event。compiler与artifact producer
   不再生成或引用这些旧public types。仍由后继D2拥有的runtime boundary/linked-type/eval legacy shape
   API可暂时保留为不可达consumer，本leaf不得为删除它们越界。
5. connect result精确为accept/reject；accept只有可选business identity/policy。wrong/nullable/generic
   return fail closed。
6. 增加`websocketConnect` external surface与typed execution plan。HTTP entry仍必须有handler；
   WebSocket entry可以无handler且adapter args为空。
7. compiler-owned `GatewayEntryKey = "websocket"`；canonical internal entry ID按现有identity framing由
   exact `serviceId + key`产生，并有language-neutral golden vector。不得复用旧operation preimage。
8. entry只进入ServiceDeployment gateway entries/ingress与RuntimeAssembly gateway ingress，不进入
   ServiceContract、service operation或`ContractOperationId`。
9. 删除`reject_unwired_websocket_authoring`的总拒绝，保留对legacy shape的精确负例。
10. 修复授权范围内所有同类legacy projection残留；不得保留compiler/authoring兼容alias。Result必须列出
    后继D2精确拥有、当前producer已不可达的runtime legacy allowlist，不能把它们误报为已清理。

## 完成标准与验证

至少证明：

- path-only无handler生成一个exact binding；
- private connect callable与两种source成功；
- malformed/legacy/multiple/generic/wrong signature/HTTP source全部拒绝；
- no-handler只对WebSocket合法；
- gateway/entry/deployment/assembly identity稳定且cross-language framing不分叉；
-新增WebSocket不改变ServiceContract与ServiceProtocolIdentity；
- 四个send native signature和`may_suspend=false`不变。
- production authoring/compiler反向搜索不再产生legacy receive/message surface；runtime D2 allowlist之外
  的同类残留为零。

运行实际匹配的聚焦测试并记录discovery：

```bash
cargo test -p skiff-artifact-model -p skiff-artifact-identity \
  -p skiff-compiler-input -p skiff-compiler -p skiff-deployment
cargo check -p skiff-runtime-loader -p skiff-runtime-linker -p skiff-runtime-host
cargo fmt --all -- --check
git diff --check
```

若实现要求修改wire、Router或runtime connect执行，返回`TASK_SCOPE_EXPANDED`；不得吞并后继节点。

## 交付

提交implementation，再新增并提交
`P5-F425A-skiff-websocket-authoring-compiler-checkpoint-result.md`。返回commit/tree、自验收矩阵和clean
worktree。不得merge/rebase/push/stable/live。
