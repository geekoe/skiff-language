# P5-I02：Skiff Consumer Combined Probe Result

结论：FAIL。唯一smoke运行一次，exit 0但未满足I02合同；不作R02 verdict。

- production candidate：`c59b4baf9752147cc49c141d89642d8b7f5aa507`
- production tree：`08051c65166eec977748b5b58c4636d26cb5eff4`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`
- docs HEAD：`f50cd78663e2c1bfeb5c802734d8ee378580b4a0`

命令约73.1秒，只执行一次single-generation activation与WS marker，输出generation 1、one replica、assembly及
`P5-F23D-REAL-COMPONENT-MARKER`。未执行/输出activationId、exact replicaId、tampered candidate reject/abort、
committed tuple与旧result不变、pending/staged归零、request artifact I/O=0或actor/spawn typed response。

阻塞属于I02 evidence entry/test-infrastructure implementation，不是已证明的production缺陷。temp Cargo target、
isolated workspace、PIDs与动态端口均清理；HEAD/tree/lock/status不变，R05B/I34证据仍有效。必须由D44先闭合可复用
入口与最小实现DAG，再修复、combined并重新运行I02；不得重复旧smoke冒充PASS。
