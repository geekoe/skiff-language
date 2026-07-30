# P5-D89：Native Route / Required Context Parity 审计

状态：Ready（只读）

## 父节点

- `P5-F155-date-from-epoch-native-semantics-result.md`
- 相关来源：`P5-F153-http-request-native-semantics-result.md`

## 目标

- 检查native signature required context、callable semantics required context、runtime route与handler实际依赖的各自owner。
- 精确解释为何request作为显式参数的headers/cookie由Http route实现但不需要HttpClient context。
- 判断validator应比较哪些canonical维度；只允许exact exception/模型修正，不按route前缀或模块泛化。
- 覆盖需要HttpClient的outbound stream/sse与不需要context的request accessors正负矩阵。
- 返回READY_TO_IMPLEMENT或最小用户决策。

只读，不修改、不提交、不操作stable。

