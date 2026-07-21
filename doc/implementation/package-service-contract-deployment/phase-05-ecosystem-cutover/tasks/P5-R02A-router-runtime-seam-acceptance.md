# P5-R02A：Router / Runtime Seam Acceptance

未参与F03A实现的独立只读Agent。输入是F03A合流后的exact clean integration commit/tree、F03A证据、
R02预审报告与权威设计。不得修改文件、创建commit、实现consumer或给R02最终verdict。

必验：

- binary runtime frame只有一个assembly control framing，direction/payload/text mutation fail closed；
- canonical assembly `request.start` TS/Rust字段、互斥legacy规则和generation/ingress值域精确一致；
- internal ecosystem-store adapter真实委托T01 typed store/identity/CAS，empty bootstrap幂等且不覆盖坏/既有state；
- snapshot读取exact ref并重算assembly/contract identity，无raw path/latest/common artifact envelope；
- public seam足以让F03B/F03C不再修改共享wire或重写storage规则；
- `extra-review`检查codec、store adapter、fixture是否出现重复parser、巨型混合职责或反向依赖。

第一行`PASS`/`FAIL`。PASS只解锁F03B/F03C，不表示R02通过。
