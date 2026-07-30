# P5-F423 HTTP authoring current migration batch

状态：Ready（WebSocket决策前可独立完成的共同checkpoint）。

## 直接父节点

- `P5-F421B-suspension-relay-first-ecosystem-proof-result.md`
- `doc/architecture/gateway-runtime-adapter-boundary.md`

F421B已证明current CLI在AIHub的旧`http.routes` sequence处fail closed。当前HTTP identity与adapter
模型已冻结；AIHub与Agine都由一个现有`HttpRequest -> HttpResponse` dispatcher按method/path处理，
因此这里只迁移manifest binding，不拆业务handler、不改变路由行为。

WebSocket业务消息路由仍待用户决策。本批次必须保持两个service的`websocket` block、WebSocket
source、API与tests原样，不删除、不改写、不新增兼容；后续publish仍可在current compiler的明确
WebSocket拒绝处停止。这不是本批次失败。

## 共同current shape

每个旧route变成唯一named HTTP entry：

```yaml
http:
  <gatewayEntryKey>:
    method: <METHOD>
    path: <PATH>
    kind: rawHttp
    handler: <current-package source selector>
    adapterArgs:
      - param: request
        source: { kind: http.request }
```

规则：

- entry key是service内稳定、唯一、camelCase identifier；
- host继续使用current默认`"*"`，不重复写入；
- handler直接绑定现有private/current-package dispatcher；
- `operation`、`routes`、旧`handlerArgs`为0；
- HTTP external ingress不进入`serviceCalls`，不生成`ContractOperationId`；
- `timeout`与WebSocket block保持原值。

两个leaf写入范围互不重叠：

```text
F423A  aihub/service/**
F423B  agine/service/**
```

leaf各自提交Internals implementation和独立Skiff result，不merge/rebase/push/stable/live。全部合流后
由新的combined节点运行一次fresh publish；leaf不重复执行完整F421。

