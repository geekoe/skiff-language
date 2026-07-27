# P5-F440A External manifest三仓owner审计

状态：Ready。高风险、只读审计。

## 直接父节点

- `P5-F440-external-manifest-and-bidirectional-websocket-batch.md`

只读本任务；需要语义时沿父节点直接引用读取。禁止把历史task/result当作权威schema。

## 目的

从三个clean integration输入列清把HTTP/WebSocket authoring从`service.yml`硬切到`http.yml`/
`websocket.yml`所需的全部production、fixture、tooling和service source owner，形成互斥、可执行的实现DAG。
本任务不实现。

## 只读范围

Skiff：

- `compiler/**`、`artifact-model/**`、`artifact-identity/**`、`deployment/**`
- `runtime/**`、`router/**`中manifest-derived gateway DTO/loader
- `test-runner/**`、`scripts/**`、`cross-system-fixtures/**`
- 所有production/test fixture root中的`package.yml`、`api.yml`、`service.yml`、`config.*.yml`

Internals与skiff-packages：

- 所有service/package root的上述control files与直接source/test/receipt/workflow owner
- 只读构建/发布/receipt脚本中source-file discovery、hash、watch、copy、temporary-root逻辑

唯一允许写入是本leaf result。禁止修改design、production、test、fixture、stable/live。

## 必须回答

1. 列出strict authoring DTO、YAML reader、source root classifier、compiler input与typed gateway projection
   的精确文件/符号；说明`service.yml`、`http.yml`、`websocket.yml`分别在哪一层读取。
2. 列出PackageArtifact、ServiceContract、gateway projection、ServiceDeployment、source revision和
   watch/build cache的hash/preimage owner，证明external manifest变化不会改变前两者，却会使后续记录失效。
3. 列出所有会复制、监听、发现或校验control file的CLI/watch/test-runner/temporary authoring helper；
   特别检查只枚举`service.yml`的硬编码。
4. 对三个repo的每个service root做完整矩阵：
   - 当前HTTP shape、entry数、kind、guard/pre、stream；
   - 当前WebSocket path/connect/legacy receive/jsonRpc；
   - timeout当前所在文件及目标`config.<profile>.yml`；
   - 目标`http.yml`/`websocket.yml`文件与需要更新的direct receipt/test。
5. 列出所有inline YAML fixture、golden artifact、generation常量与fail-closed负例。区分：
   必须刷新、应保留的legacy rejection、历史result不改写。
6. 冻结实现DAG与互斥写集，至少分为：
   - strict DTO/root discovery + compiler typed projection shared checkpoint；
   - deployment/artifact/identity follower；
   - Skiff fixture/test-runner/tooling follower；
   - Internals service migrations（按不重叠service root拆分）；
   - official packages migrations；
   - 单一combined owner。
7. 每个后继节点给首次failing test、focused selector、reverse search、身份/receipt证据及遮挡关系。
8. 如果发现`service.yml`仍被设计为deployment timeout owner、PackageArtifact必须读取external manifest、
   或新增文件会改变尚未冻结的public schema，返回`TASK_NOT_EXECUTABLE`并列证据，不自行改设计。

## 交付

- worktree：`/Users/geek/workspace/skiff-p5-f440a-external-manifest-audit`
- branch：`codex/p5-f440a-external-manifest-audit`
- result：`P5-F440A-external-manifest-owner-audit-result.md`

新增并提交唯一result；返回commit/tree、三仓root矩阵、owner/DAG、验证矩阵和clean状态。不得派子agent。
