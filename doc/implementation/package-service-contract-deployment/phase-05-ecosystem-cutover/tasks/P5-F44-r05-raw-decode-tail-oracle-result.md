# P5-F44：R05 Raw Decode and Tail Oracle Result

结论：COMPLETE。

- task commit：`9eb6b347f2683a6cc2c2afe29a5db41c9d0d3452`
- integration commit：`c59b4baf9752147cc49c141d89642d8b7f5aa507`
- integration tree：`08051c65166eec977748b5b58c4636d26cb5eff4`
- Cargo.lock blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`

HTTP 200现在保留最多512-byte raw Buffer并由F42 shared codec按固定string schema解码；JSON 200为missing-magic
负例，non-200才转UTF-8并脱敏。health尾段冻结ACK`0→0→0→1→2`、pin`0→1→2→1→0`及每步
inFlight=0/pending null；decode primary不会被finally cleanup failure覆盖。

合同node check及direct 22/22 PASS，反向搜索确认consumer无`JSON.parse(response.body)`且只有一个SKPV parser。
未运行真实probe；只解除I33。
