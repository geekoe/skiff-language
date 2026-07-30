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

## 首次验收记录

`a7566bb2619ea43f88683ce2f83b4fc4bb441c94` / tree
`b7b09ffada78952ede641469db817d6ba29478c3` 的限定gate全部通过，但验收为FAIL：canonical
assembly `request.start` 的optional fields在TS/Rust存在不同接受集合。已确认反例包括identity pattern与
`deadline` unknown/负数/小数；现有31个mutation未证明全部optional/nested字段精确一致。下一次只在D03矩阵、
F03A1 exact repair commit与失效的request parity gate上窄复验；frame/store证据不因该repair机械失效。

## 第二次验收与熔断记录

`571549739239ca16b04d09cd7be1716125dc1982` / tree
`971fa7ad9a63c4b2296b0f7b9ae8e164bcbd02ee` 的既有6个Rust tests、Router type-check、4 accepted / 244 typed
reject / 5 raw duplicate / 4 equivalent / 1 legacy self-test均通过，但验收仍为FAIL：raw lone surrogate在TS接受、
Rust拒绝；opaque unsafe integer发生TS精度丢失；四组absent/default虽双端接受，decoded typed result未统一
materialize。按验收熔断规则，D06已对剩余raw lexical、opaque number与default normalization做有界审计；
F03A2合流并通过request combined probe前不得发起第三次verdict或解锁F03B/F03C。

## 第三次验收记录

`4df6c04fe23e34f60c795ff577406cf547b127ba` / tree
`a2ee0c38fee896ea372f49a1c411c5f198fec131` 为 `R02A PASS`。29个raw cases、opaque number normalization、
四组decoded defaults、TS单次strict parse与legacy non-regression均通过；Rust 6/6、Router type-check、wire
self-test `4/244/29/4/1`、protocol 41/41全绿，candidate前后clean。该PASS冻结F03A2 shared request基线并
解锁F05；按当前DAG，F03B/F03C还必须等待F05的R05 PASS，不表示R02最终通过。
