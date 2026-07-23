# P5-R24：F05 WebSocket ABI / Owner Checkpoint

未参与F23–F27实现的全新只读Agent。输入为R25、R27、I27、R29均PASS的exact clean candidate。不得编辑、
修复、提交、运行full/I16/Host/stable或给R05/R02/Phase verdict。

必验正常source→typed ABI→wire→Runtime boundary→production Router component marker；四对象schema与frozen ABI不变；
registry/dispatcher/response/projector/lifecycle各有唯一owner；Cookie/URL/repeated metadata、zero-byte Context、response mutations、
identity/sender/direct-send错误、receive serialization/backpressure/close/shutdown均正反闭合；Assembly tests不得注入fake
registry/dispatcher。extra-review确认没有第二dispatcher/projector或新增巨型混合职责。

第一行仅`R24 PASS`或`R24 FAIL`。PASS只证明F05 ABI/owner/materialization checkpoint并解锁F23E及F03B/F03C，不证明A/B generation
lifecycle，也不改判R05 FAIL。
