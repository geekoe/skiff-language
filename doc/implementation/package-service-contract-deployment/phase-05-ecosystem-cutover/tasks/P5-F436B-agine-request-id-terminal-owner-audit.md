# P5-F436B Agine legacy requestId terminal owner audit

状态：Ready。中高风险只读路径闭合审计。

## 直接父节点

- `P5-F436-request-id-terminal-cutover-batch.md`

父节点记录当前终态、精确候选、已知 residue 与遮挡关系；引用链继续到唯一权威设计。

## 读取与写入边界

读取精确 Internals candidate 中：

- `agine/service/**`
- `agine/protocol/**`
- `agine/client/**`
- `agine/host/**`
- `shared-client/**`
- 与上述生产入口直接关联的 tests、E2E helpers、docs 和 package scripts

为区分平台内部 correlation，可只读核对 Skiff 当前 WebSocket connect/downlink wire；不得把
Router↔Runtime/control/actor request identity 纳入 Agine 删除范围。

禁止修改任何 production、test、fixture 或配置。唯一写入是本 leaf result。

## 必须回答

1. 枚举上述范围全部 `requestId`、`request_id`、`correlationId`、`correlation_id` 命中，
   按下列类别逐项归属：
   - 旧 WS req/res 匹配，终态删除；
   - 已完成 HTTP 的 negative/fail-closed test，终态保留为负例；
   - 真正业务资源/run/tool/attempt identity，保留且不得误改；
   - 第三方协议自有字段，说明 owner；
   - 当前名字错误但确有异步 job/command/result 关联需求，给出已有生命周期证据。
2. 从真实入口列出仍会让客户端 text/binary frame 到达 service receive/业务 dispatch 的每个跳点，
   并与 Router 当前 `1003` fail-closed 语义对照。指出所有 dead production source、API export、
   manifest authoring、测试 fixture 与 checker。
3. 列出 WebSocket 下行仍必须保留的业务事件及 producer/consumer；证明删除 receive/RPC 后无需
   `requestId` 匹配。ping/pong/close 归协议栈，不得保留业务 `ping` RPC。
4. 单独追踪 Host：
   - activation/hello/presence/tool-attempt/tool-result/current-directory/file-list/file-search；
   - 每条流的方向、认证来源、producer、consumer、重试/超时和结果归属；
   - 哪些可直接改为已有 HTTP endpoint，哪些需要新 HTTP entry；
   - Host file 两跳若仍需要异步 handle，判断已有 model 中哪一个稳定 identity 是 owner，或是否
     必须明确新增 `jobId`。不得继续称为 transport `requestId`。
5. 核对 browser、Host 与 shared-client 是否仍实例化 `request()`、pending map、
   `originalMessage.requestId` 或 GlobalErrorHandler 特例；给出删改闭集和不应受影响的纯下行
   `send`/listener surface。
6. 核对 Agine current `service.yml` 应收敛到最多一个 WebSocket entry、path 与可选 connect；
   不得保留 `routes`、`operation`、receive 或 API-exported websocket callable。
7. 给出一次可执行的 repair DAG：
   - 每个 leaf 的 production/test 写入 owner互不重叠；
   - 依赖顺序、共享 checkpoint、最小正/负探针和完整 combined owner；
   - 哪些旧 source 应删除而不是留 compatibility shim；
   - 反搜 gate 必须区分 explicit HTTP negative fixtures、第三方协议和 Skiff 内部 wire。
8. 若实现必须先决定尚未冻结的 Host auth credential、HTTP callback/polling 方向或 job lifecycle，
   返回 `TASK_SCOPE_EXPANDED`，列出最小决策问题与现有代码证据；不要替用户选择。

## 交付

- Skiff worktree：`/Users/geek/workspace/skiff-p5-f436b-agine-request-id-audit`
- Internals worktree：`/Users/geek/workspace/internals-p5-f436b-agine-request-id-audit`
- 分支：`codex/p5-f436b-agine-request-id-audit`

新增并提交 `P5-F436B-agine-request-id-terminal-owner-audit-result.md`，包含完整 owner/path矩阵、
Host 决策分类、repair DAG、精确输入 commit/tree、命令与两个 clean 状态。不得修改代码、运行
build/dev/start/stable/live/fixed-port workload、merge、rebase或push。完成本审计后不得承接 repair。
