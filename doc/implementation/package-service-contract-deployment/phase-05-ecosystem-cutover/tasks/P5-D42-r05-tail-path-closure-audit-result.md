# P5-D42：R05 Tail Path Closure Audit Result

结论：COMPLETE。第三次真实probe前必须批量完成F42、F43、F44及I33；不得继续逐项试跑。

D42确认production unary已正确返回canonical `SKPV` RuntimePayload。repo现有唯一JS decoder位于
`router/tests/helpers/runtimePayloadCodec.ts`，但依赖Router test helper，scripts不能按正确依赖方向消费；复制parser
不可接受。现有pin health也不能单独排除release error或disconnect/reconnect假阳性，需增加按runtime connection累计的
exact matching release ACK counter。

冻结DAG：

```text
F42 codec single-owner ─┐
                       ├─► F44 raw decode + tail oracle ─► I33 ─► 第三次probe
F43 release-ACK diag ──┘
```

F42/F43可并行且写入互斥；F44依赖二者。所有节点都是implementation work，不改变release wire、pin/activation语义、
四对象或公共HTTP representation。若要求Router把RuntimePayload转JSON、fixture绕开codec、通用SDK schema或公开
generation-indexed retired context，必须升级为设计决策。

证据锚定production `8c832b44a49b31da393064ab2c6c7d432db70274`、tree
`9f55ccc9afc87b4d3d350e3dd416f5150149e343`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。
