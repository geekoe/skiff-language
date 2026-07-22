# P5-F23D：Assembly WebSocket Convergence Result

状态：implementation checkpoint；未完成，不解锁R24。

- implementation：`fb431c204da6a774facdca717f3ad2ed819c3c9c`
- integration：`11e298ac834cc2a05a966e3fcb0ae8042223877d`
- tree：`d2c036f09b774012663827a3cd9cc1a142ae7305`
- `Cargo.lock` blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

Assembly gateway已消费F23A dispatcher receipt、F23B lifecycle、F23C strict response与shared metadata owner，删除重复
registry/socket lookup、connection/business index、policy、queue/abort、downlink和close/shutdown。Router聚焦`99/99`、
Rust F05 filters `2/1/6/7/6`、runtime wire、typecheck与diff-check PASS。

唯一真实isolated smoke一次越过compiler/deployment/activation后在connect返回502：
`parameter materialization failed: runtime value does not match the canonical contract type`。Agent拒绝使用protocol peer
workaround；根因交D35/F24 repair wave。该checkpoint不构成F23D/R24 PASS。
