# P5-F445H-I7R cross-boundary readiness preflight

状态：Ready。I7 的前瞻只读预检；与I6实现并行准备真实source、artifact、Router与consumer任务，
不提前实现、不修改任何repository。

## 直接父节点

- `P5-F445H-I6R-current-scope-refresh-preflight-result.md`
- `P5-F445H-I6S-service-timeout-scope-reduction-result.md`
- `P5-F442C-cross-system-corpus-verifier-closeout-result.md`
- `P5-F443B-cheap-combined-executable-resume-result.md`
- `P5-F444C-agine-service-terminal-connect-only-cutover-result.md`

这些父节点继续追溯到唯一权威设计。本预检只刷新父节点之后的repository事实和I7执行边界；
不得重新设计package/service、HTTP、WebSocket、Actor、internal stop或service timeout。

## 固定 Skiff 输入

```text
Skiff integration commit  a45389c6083ddd5b57b6d2ed202c1b3816f8f468
Skiff integration tree    908aee0d6a95290e8ea6dae89228bf635ce6439e
```

开始时记录该commit的完整hash/tree，并记录以下repository的当前HEAD、branch、dirty状态：

```text
/Users/geek/workspace/skiff-phase-05-integration
/Users/geek/workspace/internals
/Users/geek/workspace/skiff-packages
```

不得修改或清理已有dirty文件；无法判断归属时只报告。

I6仍在实现。I7 production节点全部blocked by最终I6 acceptance；本任务只能提前冻结owner、
真实入口、fixture、命令和可并行DAG，不能根据未完成I6接口写代码。

## 已冻结语义

- 第一版service call只继承caller current execution deadline和外层`timeout(...)`；没有独立
  dependency/callee timeout，也不复用deployment `policy.timeoutMs`。
- 没有公开request cancellation、peer `$/cancelRequest`、`-32800`或`CancelError`。
- WebSocket普通send不挂起；`requestJsonToConnection`保持三参数并由平台隐藏transport id。
- Agine service的WebSocket用途是connect与向Host发起JSON-RPC request；除非当前authoring已明确声明
  peer调用Skiff的method，否则不为“可能以后上行”增加空业务路由。
- HTTP与WebSocket ingress由`http.yml` / `websocket.yml`拥有，不放回`service.yml`。
- I6只交付hermetic runtime receipt；真实source compile、artifact/golden、Router wire、consumer与
  smoke属于I7。

## 必答问题

### 1. Skiff source → compiler → artifact → runtime → Router

从至少一个真实`.skiff` source追踪：

```text
source/config/http.yml/websocket.yml
  -> compiler/linker/File IR
  -> PackageArtifact/ServiceContract/DeploymentArtifact/assembly
  -> Host/runtime admission
  -> Router HTTP/WebSocket dispatch
  -> observable response/stream/RPC result
```

逐跳记录当前positive fixture、owner、identity版本、已有验证命令、仍缺的receipt与上游失败会遮挡的
下游。重点核对：

- nested `timeout(...)`下的HTTP、WebSocket request、file、Actor与service call真实source是否存在；
- raw HTTP response stream/codex-relay式转发能否保持status、headers和chunk顺序；
- current `http.yml` / `websocket.yml`与artifact/golden/cross-system corpus是否一致；
- runtime/router wire是否仍有legacy cancellation、receive/requestId、旧identity或legacy service relay；
- F442/F443证据在当前tree是否仍有效，哪些会被I6 production变化失效。

只读列出或展开命令；不得执行Cargo完整gate、server、network或live。

### 2. Internals真实consumer

核对`internals/agine`与`internals/codex-relay`当前production/test事实，至少回答：

- F444C报告的Agine interface identity首错是否已被后续F445C系列修复；
- Agine service当前manifest/source是否已经是connect-only WebSocket、typed HTTP入口和
  `requestJsonToConnection` outbound Host RPC终态；
- Host工具执行、文件list/search等调用的业务params/result是否仍无transport `id` / `requestId`；
- Agine client仍通过HTTP向Skiff发送请求、通过WebSocket只接收下发的边界是否成立；
- codex-relay内部service调用与对外raw HTTP stream转发的真实入口、上游stream格式和缺口；
- 哪些test是hermetic source compile，哪些依赖stable account/OAuth/browser/live，不能混为同一receipt；
- chat smoke受影响的实际跳点和最小诊断探针。

不得修改internals、安装依赖、运行OAuth、stable、浏览器或网络。

### 3. 官方packages

核对`/Users/geek/workspace/skiff-packages`中I7真实使用的official package root：

- source/API与当前Skiff std/native/type identity是否一致；
- 是否仍有旧service contract、test service、test-doubles、receive/requestId/cancel或legacy config；
- 哪些package compile/golden会被I6 carrier变化真实影响，哪些只是无关完整gate；
- package repo是否需要production修改，还是只需作为consumer gate。

不得因为“官方package可能相关”而开放式扫描全仓库；从Agine/codex-relay与现有Skiff fixture的实际
dependency边反查。

### 4. 可执行 I7 DAG

给出短共享检查点后扇出的最小DAG。至少区分：

- Skiff真实source/artifact/fixture；
- Router/runtime cross-layer receipt；
- official packages consumer；
- internals Agine service；
- internals codex-relay；
- 最终stable/chat/browser/live验证。

每个节点必须列出：

- 直接父节点与blocked-by；
- repository、production/test owner和精确允许写集；
- 禁止写集；
- 第一处预期修改或“只读gate”；
-真实RED/positive/negative receipt；
- 聚焦命令、完整命令与唯一owner；
- 可否与其它节点并行；
- commit、worktree、跨repo集成与证据失效边界。

不要把多个顶层repository塞进同一个实现任务。若某repo无需修改，明确作为只读consumer gate，不制造
空实现节点。

### 5. Gate前置预检

只读确认最终命令是否真实可执行：

- selector/list/dry-run形态与非零目标；
-依赖、工作目录、源码来源、构建缓存/target隔离；
- stable instance、MongoDB、OAuth、browser等共享状态哪些需要用户授权和唯一owner；
- chat smoke的前置runtime binary/artifact reload步骤；
- 哪些gate可hermetic提前运行，哪些必须留到最终稳定候选。

不得执行准备动作、安装依赖、启动服务或改变共享状态。

## 任务内并行

父任务Agent可以派最多三个互不重叠的只读子Agent：

1. Skiff source/artifact/runtime/Router；
2. internals Agine/codex-relay；
3. official packages dependency闭包。

每个子Agent只返回精确路径、调用链、命令、缺口与HEAD/dirty事实，不修改文件、不运行昂贵gate；
子Agent不得继续委派。父Agent统一判断DAG、遮挡关系、ready queue与是否需要用户决策。若任一分片发现
公共设计缺口，只报告，不让其它分片猜测。

## 输出

只新增：

`P5-F445H-I7R-cross-boundary-readiness-preflight-result.md`

Result必须包含：

- `READY_FOR_I7_DAG`、`DECISION_REQUIRED`、`TASK_SCOPE_EXPANDED`或
  `TASK_NOT_EXECUTABLE`；
- 三repo精确HEAD/tree/dirty状态；
- 真实跨层跳点、遮挡关系与证据矩阵；
- 最小I7 DAG、并行波次、关键路径与预计串行波次数；
- gate前置条件及授权边界；
- 当前需要用户回答的问题；没有则明确写无；
- 哪些节点只等待I6 acceptance即可启动。

```text
worktree /Users/geek/workspace/skiff-p5-f445h-i7r-preflight
branch   codex/p5-f445h-i7r-preflight
```

只提交result，最终worktree clean；不得merge、rebase或push。本任务是只读预检，启动五分钟内必须
建立result骨架和三个分片边界；若无法有界执行，立即返回精确阻塞。
