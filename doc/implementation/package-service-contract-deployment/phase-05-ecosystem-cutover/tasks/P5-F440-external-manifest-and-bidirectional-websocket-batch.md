# P5-F440 External manifest拆分与双向WebSocket JSON-RPC批次

状态：Ready。当前先执行两个owner审计和两个已闭合的独立checkpoint。

## 直接父节点与权威输入

- `P5-F439-websocket-jsonrpc-and-cancellation-batch.md`
- `P5-F439A-cancellation-public-surface-owner-audit-result.md`
- `P5-F439C-agine-host-jsonrpc-delta-audit-result.md`
- `doc/architecture/package-service-contract-deployment.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`
- `doc/reference/service-yml.md`

权威设计提交：

| Commit | Tree | 作用 |
| --- | --- | --- |
| `c2c1c41c36bce9945d617a8bd8e0eea834f5478d` | `ea7b0e56edad903bf058b73361147b0ee0395b7a` | 拆分`http.yml`/`websocket.yml`并冻结双向declared JSON-RPC |
| `1197294db8ca28c4fa0ff9992bfc7d0d28719378` | `4168cc402eba04f7eaabe4f1637e8d41139c9a40` | 冻结active/settled request id复用与双向tombstone规则 |

实现输入worktree：

| Repo | Root | Commit | Tree |
| --- | --- | --- | --- |
| Skiff | `/Users/geek/workspace/skiff-phase-05-integration` | `1197294db8ca28c4fa0ff9992bfc7d0d28719378` | `4168cc402eba04f7eaabe4f1637e8d41139c9a40` |
| Internals | `/Users/geek/workspace/internals-phase-05-integration` | `faa11b188c570ca763f107ddd829d52b8fe8861f` | `140d3a03851b64d513fd97c5860e713b8fc314de` |
| skiff-packages | `/Users/geek/workspace/skiff-packages-phase-05-integration` | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

三个integration worktree必须clean。禁止push、stable/live和从任务worktree注册稳定watch。

## 已冻结的manifest边界

- `service.yml`只拥有service id、`serviceCalls`，测试service可另有`kind: test`。
- deployment timeout等policy/value继续由`config.<profile>.yml`提供；`timeout: 30000`投影为可选
  `DeploymentPolicy.timeoutMs`。`service.yml`不再拥有timeout。
- 可选`http.yml`顶层就是named HTTP entry mapping，没有`http:`或`routes:`包装。
- 可选`websocket.yml`文件本身就是当前service唯一WebSocket entry，拥有`path`、可选`connect`和可选
  `jsonRpc` mapping。
- `http.yml`/`websocket.yml`只能随合法`service.yml`出现；旧内联字段和旧route/receive shape直接报错，
  不保留兼容读取。
- external manifest变化不改变PackageArtifact或ServiceContract；它改变typed ingress projection、
  ServiceDeployment及其后续identity。

## 已冻结的双向JSON-RPC边界

- Skiff-originated request只恢复原挂起调用；peer-originated request只有精确命中
  `websocket.yml.jsonRpc` method才创建typed gateway ingress。
- Broker核心与`jsonrpc-2.0-text` profile分层。两个方向使用独立pending/active namespace，同值id不能
  碰撞；平台outbound只生成connection-generation内不复用的非空string id。
- Peer inbound id允许非空string或safe integer并按原JSON类型回显。Active或仍在bounded settled
  tombstone中的同方向id重复时以`1002`关闭；tombstone到期/驱逐后才可复用。
- Parse error使用`-32700`和`id:null`；invalid request/batch使用`-32600`和`id:null`；合法id之后的
  method/params/internal/capacity/timeout/cancel错误回显原id。
- 第一版不执行batch、不接收binary RPC、没有raw receive、业务notification handler、event-name
  fallback或业务可见transport id。除`$/cancelRequest`外的合法notification即使method同名也不dispatch。
- Inbound handler必须绑定一次完整`websocket.jsonRpcParams`，可另取
  `websocket.connectionId`/`websocket.businessIdentity`；只能unary return。
- 预期业务失败使用typed result union。未捕获throw统一脱敏为`-32603`；平台容量、timeout、cancel分别为
  `-32000`、`-32001`、`-32800`。
- Peer cancel与disconnect触发不可捕获的结构化取消。有效cancel先固定settled状态，再best-effort回写
  `-32800`；late completion不得二次写回。

## F439C的适用范围与明确覆盖

F439C关于三项Host method、现有代码owner、授权、旧DB/browser relay删除和四个Internals互斥写集的事实继续
有效。它早于本批次双向规范，以下协议结论被本节点覆盖：

- parse/invalid/batch不关闭健康socket，而按上述`id:null`错误处理；
- 合法业务notification忽略，不以`1003`关闭；
- active与仍在tombstone中的id不可复用，tombstone过期/驱逐后可复用；
- cancel赢得执行后best-effort发送原request的`-32800`response；
- Host不得占用`-32001..-32004`表达业务错误；`-32000/-32001/-32800`保留平台容量/timeout/cancel；
- invalid path与outside workspace属于预期业务失败，进入typed result union；其它未分类Host异常脱敏为
  `-32603`。

## 第一波任务

四个任务可并行，因为写集互不重叠：

```text
F440A  三仓external manifest/parser/projection/migration owner审计（只读）
F440B  Skiff双向broker/runtime/router owner审计（只读）
F440C  CancelError compiler/artifact hard cut（compiler + artifact-model）
F440D  Agine Host peer protocol/fixture checkpoint（agine/protocol）
```

F440A/F440B只产生result，不得顺手实现。F440C/F440D是现有审计已经闭合的确定性leaf；若首次失败暴露
任务外production owner或公共语义缺口，必须停止并返回scope expansion。

## 后续DAG

```text
F440A result
  -> external manifest strict DTO/compiler/deployment shared checkpoint
      -> Skiff fixtures/tooling migration
      -> Internals services migration
      -> official packages migration

F440C
  -> F439A.R0 capability-context terminal
      -> R1 native/eval/service channel
          -> R2 request/Host/transport
          -> M0 runtime model cleanup
      -> Q0 Router cancellation projection

F440B result + manifest shared checkpoint + required cancellation checkpoint
  -> std/artifact/runtime transport shared RPC checkpoint
      -> runtime typed inbound/outbound execution
      -> Router profile-neutral broker + JSON-RPC adapter
          -> Skiff focused combined

F440D
  -> Host private peer adapter（可在Skiff shared checkpoint前单测）

Skiff focused combined + manifest migration + Host adapter
  -> Agine service typed caller/HTTP cutover + client migration
      -> Internals cross-language combined
          -> independent acceptance
              -> final gate
```

昂贵gate只允许最后一个唯一owner运行。任何leaf不得修改其它leaf的代码写集。

## 批次完成标准

- 三个manifest owner、hash/identity边界、所有source roots及watch/test/fixture consumer均有单一实现owner。
- `CancelError`从source/artifact公开面移除，内部cancel signal/control frame继续可靠工作。
- 双向broker/profile/runtime adapter不复制pending owner，两个方向、取消和generation竞态有直接证据。
- Host三项读取使用typed params/result；业务DTO无transport id，预期路径错误不滥用JSON-RPC error。
- 全部旧内联manifest、raw receive、DB/browser relay和重复correlation从production移除。
- Skiff、Internals及official packages各自聚焦combined通过，再进入独立验收和唯一final gate。
