# P5-F442E Router / Runtime README current semantics result

状态：`PASS / CURRENT_SEMANTICS_DOCUMENTED`。

Router 与 Runtime README 已删除旧 raw receive / route fallback / 自动响应、旧
build/service-assembly pointer 拓扑和旧协议代际，改为记录当前 external manifest、
RuntimeAssembly、双向 JSON-RPC 与 WebSocket writer 边界。规定静态验证全部通过；未运行测试、
stable、network 或 live。

## 1. 基线、提交与写集

| 项目 | 值 |
| --- | --- |
| implementation baseline | `0303fe5d` |
| leaf dispatch HEAD | `2989ddf97391e97b99c3c1dd8c3d9468de0d28f7` |
| worktree | `/Users/geek/workspace/skiff-p5-f442e-router-runtime-readme` |
| branch | `codex/p5-f442e-router-runtime-readme` |
| implementation commit | `588c66f55da432cf4894add73baff3316cf89068` |
| implementation tree | `7872f25bc4b3acb4afc4f4615e773f5ddfd2fc90` |
| result commit | 本文件独立提交，见 branch history |

Implementation commit 精确只修改：

- `router/README.md`
- `runtime/README.md`

本文是唯一新增 result，不混入 implementation commit。没有修改代码、fixture、checker、其它
task/result 或 canonical architecture/reference 文档。

## 2. Current 语义收敛

两份 README 共同冻结以下 current 边界：

- HTTP 与 WebSocket external surface 分别由 `http.yml`、`websocket.yml` 拥有；
  `service.yml`不再内联 ingress，deployment timeout只来自选中
  `config.<profile>.yml`的可选正整数 `timeout`；
- Router 只消费 active RuntimeAssembly 的 `globalIngress`和 compiler 投影的精确 entry facts，
  不从 rewrite、handler name、business payload或service配置推断 ingress；
- WebSocket 没有 raw receive、任意业务 route fallback或根据handler return自动响应；
- declared `jsonRpc`把 peer request投影为typed unary ingress；Skiff通过
  `std.websocket.requestJsonToConnection`反向请求peer，response只恢复原调用；
- peer业务notification被忽略，`$/cancelRequest`由平台按方向与connection generation处理；
- peer JSON-RPC id和Runtime frame `requestId`分别由profile/broker与Runtime transport拥有，
  不进入业务handler；
- ordinary direct/business `connection.send` downlink与RPC broker的captured、
  generation-bound observed writer是两条不同路径，不互为fallback；
- current generation明确为 GatewayEntry v2、ServiceProtocol v5、
  DeploymentArtifact v3、RuntimeAssembly v2；
- `std.websocket`名称逐项取自`std/websocket.skiff`：
  `sendTextToConnection`、`sendBinaryToConnection`、
  `sendTextToBusinessIdentity`、`sendBinaryToBusinessIdentity`、
  `requestJsonToConnection`、`sendJsonToConnection`和
  `sendJsonToBusinessIdentity`。

旧 published build/service-assembly pointer、dev reload pointer、version selector与manifest
fallback的大段拓扑说明已删除。没有机械替换仓库中的`publication`；本 leaf 写集只有两份 README，
因此 compiler source/resource与Host stream/commit中的 current 内部含义未被触碰。

## 3. 规定反向搜索

执行：

```bash
rg -n "receive|requestId|service-protocol-v[1234]|gateway-entry-v1|deployment-artifact-v[12]|sendText\\(|sendBinary\\(|sendJson" \
  router/README.md runtime/README.md
```

得到 8 个 allowlisted current 命中：

| 命中 | 分类 |
| --- | --- |
| `router/README.md:104` raw `receive` | 明确 negative：不存在该handler |
| `router/README.md:126` `requestId` | current Runtime transport内部id，明确不进入业务handler |
| `router/README.md:139-140` `sendJsonTo*` | `std/websocket.skiff`当前helper名称 |
| `runtime/README.md:77` raw `receive` | 明确 negative：不存在该callback |
| `runtime/README.md:89` `requestId` | current Runtime transport内部correlation，业务不可见 |
| `runtime/README.md:115-116` `sendJsonTo*` | `std/websocket.skiff`当前helper名称 |

以下模式均为 0 命中：

- `service-protocol-v1`到`service-protocol-v4`
- `gateway-entry-v1`
- `deployment-artifact-v1`与`deployment-artifact-v2`
- 旧 `sendText(`、`sendBinary(` 与 `sendJson(` 函数名

## 4. 验证与自验收

| 条款 / 命令 | 证据 | 结论 |
| --- | --- | --- |
| external manifest owner与profile timeout | 两份 README 的 source/deployment boundary | PASS |
| 无 receive/fallback/自动响应 | 两份 README 的明确 negative | PASS |
| peer与Skiff双向request、notification/cancel | 两份 README 的WebSocket语义段 | PASS |
| transport id不进入handler | 两份 README 的id边界段 | PASS |
| direct/business send与captured observed writer分离 | 两份 README 的writer边界段 | PASS |
| 四个current generation | 两份 README 的identity列表 | PASS |
| `std.websocket` source name | Router完整列表；Runtime native signature与helper | PASS |
| 旧拓扑删除且不机械改`publication` | implementation changed-path audit | PASS |
| 规定`rg` | 8个命中全部逐条分类；stale generation / 旧function为0 | PASS |
| `git diff --check`与`git diff --check HEAD^ HEAD` | 均无输出，exit 0 | PASS |
| implementation changed paths | 仅`router/README.md`、`runtime/README.md` | PASS |

本 leaf 未派 sub-agent，未运行测试，未启动或访问 stable、Router、Runtime、MongoDB、固定端口、
network或live workload；未 merge、rebase或push。
