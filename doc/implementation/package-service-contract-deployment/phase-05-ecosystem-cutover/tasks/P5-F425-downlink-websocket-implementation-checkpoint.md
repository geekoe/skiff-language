# P5-F425 Downlink-only WebSocket implementation checkpoint

状态：Implementation ready。当前不是稳定候选。

## 直接父节点

- `P5-F424-downlink-only-websocket-cutover-batch.md`
- `P5-F424A-skiff-connect-outbound-owner-audit-result.md`
- `P5-F424B-agine-http-uplink-owner-audit-result.md`
- `P5-F424C-aihub-http-uplink-owner-audit-result.md`

父链可追溯到唯一权威设计
`doc/architecture/package-service-contract-deployment.md`；WebSocket内部边界由
`doc/architecture/gateway-runtime-adapter-boundary.md`细化。叶子任务不得恢复receive/message surface。

## 精确输入

| Repo | Commit | Tree |
| --- | --- | --- |
| Skiff integration | `a0cb0e18cf7df2bdbb7f90e0072cac62fe6164fa` | `6553e04a11ac3eedb056ca360575fcf47a9b1f34` |
| Internals integration | `eddeeb8615057233a8a9ba2fbcf748d863d23e3b` | `b587fc9a7d2a7916d86c01533955955c43b9ac85` |
| skiff-packages integration | `f8c634ce4573506e35f6bc1c7cc1e4eef9992a78` | `eb00877ef260d122552af1ff0491c74102adbd57` |

F424 result提交只增加审计文档，没有production差异。任一相关production输入变化会使本checkpoint重新进入
预验收状态。

## 已冻结的实现边界

- 第一版每个service零或一个WebSocket entry。
- `service.yml` target authoring为一个严格对象：

  ```yaml
  websocket:
    host: "*"
    path: /ws
    connect:
      handler: internal.socket.connect
      adapterArgs:
        - param: request
          source: { kind: websocket.connectRequest }
  ```

  `host`默认`"*"`，`connect`可省略。没有`routes`、author id、receive/message/context或多entry容器。
- compiler-owned `GatewayEntryKey`固定为owner-local `"websocket"`；canonical internal
  `WebSocketEntryId`按现有language-neutral identity framing由`serviceId + gatewayEntryKey`产生。author和
  std native都不提供entry id。
- connect result没有Context；无connect handler由Router直接accept且零runtime dispatch。
- client text/binary data frame在gateway第一跳close `1003`，零runtime dispatch。
- 四个outbound native保持现有签名与non-suspending summary；HTTP/service/actor activation可从当前
  service deployment解析唯一entry。
- AIHub没有独立主动push，迁移HTTP event server stream后删除全部AIHub WebSocket surface。
- Agine保留真实chat/Host异步下行。Host认证、activation与host-file结果通道仍等待用户确认；不受其影响的
  普通user HTTP service迁移可以先完成。

## 当前DAG

第一波互不重叠：

```text
F425A  Skiff strict authoring/artifact/compiler checkpoint
F425B  AIHub service raw HTTP server stream + remove WebSocket
F425C  AIHub browser Fetch/SSE/Abort migration
F425D  Agine non-Host/non-host-file user HTTP service checkpoint
```

后续关键路径：

```text
F425A -> Skiff current connect wire
              -> Runtime/Host consumer ----+
              -> Router consumer ----------+-> fixture/oracle convergence
                                            -> connect/downlink cheap combined

F425B + F425C -> AIHub combined

用户冻结Host方案
  -> Agine Host contract
  -> Host service + Host caller + host-file job/poll

F425D -> Agine ordinary browser callers
all Agine callers migrated -> legacy receive cleanup
all branches -> fresh N5
```

## 成熟度与证据边界

F425A是共享实现检查点；F425B/C/D是consumer局部检查点。开发leaf只运行聚焦验证，不运行完整N5、
stable/live/instance或最终gate。每个leaf分别提交自己的repo改动与result，不merge/rebase/push。

所有leaf合流后先运行一次便宜combined probe；只有production owner闭合、无在途写入且相关probe通过后，
才能冻结新的N5候选。

