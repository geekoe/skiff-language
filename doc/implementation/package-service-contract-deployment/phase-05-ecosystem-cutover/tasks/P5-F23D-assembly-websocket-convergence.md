# P5-F23D：Assembly WebSocket Convergence

依赖F23A/F23B/F23C exact commits全部合流。独占`assemblyWebSocketGateway.ts`、server wiring、shared request metadata reader、
F05 real-smoke fixture/caller与直接tests；消费前三个接口，不回改其owner，也不实现F03B/F03C generation registry。

Assembly adapter删除registry直依赖及重复index/policy/queue/downlink/close，消费dispatcher receipt和shared lifecycle core；
connect/receive使用唯一strict response decoder。connect metadata复用shared query/raw-header/cookie owner，拒绝absolute-form
authority/scheme/credentials/fragment与routing不一致；保留ordered repeated fields。direct send绑定sender runtime、service、entry、
connection且错误可观察。删除过期real-smoke authoring blocker，checked-in normal-source fixture必须从compiler/deployment进入
真实Router registry/dispatcher/runtime protocol peer并到client marker；测试不得fake registry/dispatcher。

运行F05原targeted矩阵、shared lifecycle参数化suite、真实production-component正反例与diff-check；不声称B后旧A，禁止
full/I16/Host/stable。一个clean commit，不merge/push。
