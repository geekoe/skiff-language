# P5-R05A：Canonical WebSocket Ingress Reacceptance Result

结论：FAIL。唯一真实命令运行一次，未试跑、未重试、未修改候选、未操作stable。

- production candidate：`8c832b44a49b31da393064ab2c6c7d432db70274`
- production tree：`9f55ccc9afc87b4d3d350e3dd416f5150149e343`
- docs HEAD：`76566679784b27aef7e94754080431fd87738fd7`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

真实transcript完成A/B authoring、A activation/connect、B activation、A receive×2 A marker、B connect/receive B
marker及B unary wire HTTP 200。production response以canonical `SKPV` RuntimePayload magic开头；harness把任意
response bytes转UTF-8后直接`JSON.parse`，因此报`Unexpected token 'S'`。

当前最小反例属于scripts/test-infrastructure unary response client/oracle/direct evidence：I32的success server固定返回
JSON，未覆盖production codec。不得新增重复ABI parser，必须查找并复用canonical codec owner。因B marker未解码确认，
`close B → pin=1 → close A → pin=0/inFlight=0/no pending`仍未执行。

这是同一路径修复后暴露的第二个新blocker，触发收敛熔断。第三次完整transcript前必须先完成D42路径闭合审计、批量
修复及合流状态cheap combined；不得直接继续逐项修补。
