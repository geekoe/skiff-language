# P5-F442E Router / Runtime README current semantics

状态：Ready。非阻塞文档节点；删除旧receive/route/publication拓扑说明并记录current边界。

## 直接父节点

- `P5-F442A-final-fixture-tooling-preflight-result.md`
- `P5-F440Z3E-router-websocket-rpc-gateway-integration-resume-result.md`

实现基线为 `0303fe5d`。

## 唯一写集

- `router/README.md`
- `runtime/README.md`
- 本节点result

禁止修改代码、fixture、checker、其它文档或task/result。

## 内容要求

两份README应准确说明：

- external HTTP/WebSocket surface分别由`http.yml`/`websocket.yml`拥有；
- `service.yml`不内联ingress，timeout来自profile config；
- WebSocket没有raw receive、业务route fallback或自动响应；
- declared `jsonRpc`支持peer与Skiff双向request，peer notification忽略，
  `$/cancelRequest`由平台处理；
- peer JSON-RPC id与Runtime frame requestId均为transport内部，业务handler不感知；
-普通direct/business `connection.send` downlink与RPC captured observed writer是不同路径；
- GatewayEntry v2、ServiceProtocol v5、DeploymentArtifact v3、RuntimeAssembly v2；
- current `std.websocket`函数名以source为准；
-删除没有额外current信息的旧published build/service-assembly拓扑段落。

不得机械全文替换`publication`：compiler source/resource与Host stream/commit中仍有current内部含义，
只删除已无current信息的旧发布拓扑描述。

## 验证与交付

```bash
rg -n "receive|requestId|service-protocol-v[1234]|gateway-entry-v1|deployment-artifact-v[12]|sendText\\(|sendBinary\\(|sendJson" \
  router/README.md runtime/README.md
git diff --check
```

剩余命中必须逐条解释为current transport/internal term或明确negative。不得运行测试、stable、
network或live。

- worktree：`/Users/geek/workspace/skiff-p5-f442e-router-runtime-readme`
- branch：`codex/p5-f442e-router-runtime-readme`
- result：`P5-F442E-router-runtime-readme-current-semantics-result.md`

Implementation与result分开提交。5分钟内开始修改；不得派子Agent，不得merge/rebase/push。
