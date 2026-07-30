# P5-R11：Runtime Committed Bootstrap Acceptance

未参与F10实现的独立只读Agent。输入为F10 exact clean commit/tree、D12/F10合同、R10 PASS seam与F04保留的cold-start
失败证据；不得编辑、提交、修复或给F04/R02 verdict。

必验：

- Runtime config只有一个strict environment/singular artifactRoot owner；全部renderer caller显式传值，旧plural/multi-root/
  missing/empty fail closed；
- 每次连接前从exact durable path重读并完整验证committed state/ref/content，执行production resolve/load/link/admit，
  失败时零次连接；不得扫描、猜latest、直接写store或接受pending；
- cold recovery与online prepare/commit复用同一admission/publication primitive；active与committed原子发布，无第二
  direct-admit owner；
- capabilities后exact committed register，pending仍由Router重放；reconnect能观察N→N+1且不stale register；
- generation-0只走recovery，online transaction、abort/commit/register语义未放宽；
- 未修改Router/shared codec/test fixture/F05/manifest/lock，未提前实现request trust boundary、WS或drain；
- `extra-review`确认lifecycle是唯一reconnect owner，未向大`router_session`/`control_plane`复制startup workflow；并运行
  F10全部静态、动态与真实ready-only门禁。

第一行只给`R11 PASS`或`R11 FAIL`。PASS只允许在exact合流状态恢复F04A真实Host probe；ready-only、Runtime单测或
register字符串不能替代后续F04 checked-in consumer Host结果。

## 验收记录

`R11 PASS`：exact candidate `47d92595cc346cdbbee184ebb467f3bc2aecb01d` / tree
`70d3c8d31c2a748ff642c99f2f3c29947bf181c2`，parent `028154dcdb18fde954e0e8b8a42052419d128133`，single
commit/clean/lock不变。真实ready-only health同时出现exact gen0 active、capability connection与healthy replica。
required Clippy不记PASS：base/candidate均有既有warnings，candidate总数下降且F10新模块无诊断。
