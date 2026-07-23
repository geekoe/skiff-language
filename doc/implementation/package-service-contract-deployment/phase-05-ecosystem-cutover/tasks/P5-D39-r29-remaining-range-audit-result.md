# P5-D39：R29 Remaining-Range Audit Result

状态：complete。锚点production candidate为`8982107308c021fe9a72ad9446e1820395a0bc83`、tree
`f7457b1d11a43406763184e8ff220277d6ac6049`、Cargo.lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。R29只运行一次，在bootstrap strict identity处FAIL；actual std build为
`2541456b...`，JS oracle仍期待c277e45前的`3bbab8df...`。

D39A identity传播审计确认production只有一个真实owner：
`CompilerPlatformSources`→compile→F27A typed publication receipt→F27B seed/CAS receipt→bootstrap JSON。F27C oracle与fake
fixture另建旧build事实源，使Node测试自洽并遮挡真实Rust→JS交接。typed receipt已足够校验coordinate、identity framing、
artifact/pointer/record path关系，不需要新增公共字段或硬编码当前hash。另有两个active compiler prelude tests仍pin旧
`aae18f07...`，虽不阻塞R29入口，但会阻塞后续full gate。

D39B从identity检查之后审计到cleanup，未发现正常路径静态必失败证据；真实跨进程
commit/re-register→WS receipt→runtime wire→Event/Result→native direct-send仍只能由完整smoke观察。独立生命周期blocker是
activation fetch、WS open和WS close没有覆盖整个run的signal/deadline；任一Promise悬挂都会阻止outer isolated cleanup开始。
这属于smoke harness owner，不改变Router/runtime、业务retry或公共contract。

批量修复DAG：

1. F28A独占smoke identity oracle/fixture/tests，删除手写build事实源并准备actual bootstrap→JS oracle便宜交接探针。
2. F28B依赖F28A，独占smoke I/O lifecycle及同一直接Node tests；给activation、open、close/terminate建立共同deadline。
3. F28C独占两个compiler source prelude test pin，可与F28A并行；不改production prelude算法。
4. F28A/B/C合流后I28只运行受影响combined。PASS后R30才可运行一次完整real smoke。

F28A与F28B共享`package-service-ecosystem-smoke-real.test.mjs`，保持真实串行；F28C写集互斥。下一次完整探针仅用于D39确认
无法由mock/static/Cargo-only probe替代的跨进程尾段，失败不得原地重试，必须重新分类。
