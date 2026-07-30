# P5-F03B：Router Integration Repair Result

> **历史结果，ingress verdict已失效。** D0改为service/version header先选精确deployment、再在service内
> 选route，并升级Deployment/RuntimeAssembly/runtime-frame代际。因此`a18c3d1`中route selection、
> header/global-map及“10 files/64 tests PASS”的总体ledger不能证明新契约。Store、participant、
> generation pin/drain只保留为历史实现事实；在新代际合流后仍需受影响回归。

状态：complete。commit `a18c3d14f6dd006de3da89b0ada42a9310b3654a`、tree
`749d73a806dcb21cc05f5510f04ade560b0e983f`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。

Router已通过唯一compiler adapter消费canonical store，删除Node file lock/CAS/path owner；capability connection成为
activation participant来源，HTTP按exact ServiceContract选择unary/serverStream，WS完成generation pin及F23E
acquire/release/ack/reject、disconnect cleanup与可观测drain。10个direct files/64 tests、type-check与diff-check PASS。
F23E shared wire、Rust与Cargo.lock未改。

extra-review未发现blocking finding。独立范围外缺口是instance/deploy脚本尚未安装并写入Router
`ecosystemStoreCliPath`；它不回退给F03B，交D40审计。
