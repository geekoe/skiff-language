# P5-R05：Canonical WebSocket Ingress Acceptance

未参与F05实现的独立只读Agent。输入为F05 exact clean integration commit/tree、D05/F05合同、R02A PASS seam与
聚焦证据；不得编辑、提交、修复或给R02最终verdict。

必验：

- 正常source/compiler/deployment路径产生typed unified WS ABI，四对象schema无变化、artifact无patch/re-sign；
- TS/Rust `websocket.ingressEvent`接受集合、payload segment/context presence/default normalization一致；legacy
  adapter不因新enum放宽；
- runtime只经pinned boundary descriptor与`dispatch_in_process_boundary`，connect/receive返回规则fail closed；
- production direct connection send可观察旧A连接在B激活后仍执行A，新连接/unary使用B；无ambient state/test hook；
- Router entry/gateway identity稳定且隔离service/entry，business index/policy/close/drain owner唯一；
- `extra-review`检查std/compiler/wire/runtime/Router间无重复ABI parser、第二dispatcher或巨型混合职责。

第一行只给`R05 PASS`或`R05 FAIL`。PASS解锁F03B/F03C；FAIL给最小production反例、失效证据与唯一owner。
