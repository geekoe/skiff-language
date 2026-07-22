# P5-D35：WebSocket Builtin Materialization Closure Audit Result

状态：complete；candidate `11e298ac834cc2a05a966e3fcb0ae8042223877d`保持只读clean。

首错位于`runtime/boundary::builtin_value_matches`：Event参数在detached clone前TypeMismatch。若只补Event，非null
ConnectResult仍在同一matcher失败；receive null return仅因Nullable短路可过。nominal Context另有确定性阻塞：compiler
按设计把contract execution leaf擦为Unknown，而canonical eval错误地从executable取得event/result nested Context plan，
导致receive decode与accept encode失败。现有正例只覆盖Context=null。

同一service-value matcher还缺safe integer、Duration、JsonObject、Date范围、representation-over-string Map key与legacy
Json metadata拒绝；这些均可出现在合法nominal Context中。CallbackInterface Context虽有nominal id，不能持久化为bytes，
必须在admission fail closed。linked-type-plan与boundary test descriptor已有两份WS shape constructor，禁止再加第三份。

冻结修复顺序：F24A建立唯一内部WS shape spec与Context admission，R25接收；F24B从pinned contract编译统一service-value
plan并关闭完整contract closure，F24D并行收敛shape parity；F24C再让eval从pinned ServiceContract取得nested Context plan。
I24只跑cheap combined，R26只运行一次真实isolated smoke，PASS后R24才验收owner checkpoint。无需用户设计决策。
