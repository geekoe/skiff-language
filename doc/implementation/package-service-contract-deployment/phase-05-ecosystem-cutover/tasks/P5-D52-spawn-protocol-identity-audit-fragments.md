# P5-D52：Spawn Protocol Identity Audit Fragments

冻结candidate `e3b93c4ef6907d59e3a58e7ab17448ccec34c4d0`。两个分片一致确认compiler/artifact/
RuntimeAssembly产生canonical `skiff-service-protocol-v2:sha256:<64hex>`，I02E由Router runtime control wire错误
使用legacy `skiff-protocol-v1` validator导致；禁止改producer、dual-prefix、fallback、重算或identity inference。

待汇总范围：

- D52A确认当前I02 blocker至少覆盖Router spawn submit、claim request及claim response item三处旧pattern。
- D52B另发现runtime.register、renew/complete/fail同族、host loader/register mapper与大量手写v1 fixture；
  现有测试因人工值自洽而遮挡真实projection断裂。

汇总owner必须枚举每个字段的语义类型，区分runtime protocol identity与service protocol identity；只把确实承载
`ServiceProtocolIdentity`的production admission/wire面纳入同一修复波次。给互不重叠写入owner、共享真实identity
探针、关键v1负例及证据失效面。若设计已明确不得缩成只修首个报错；若字段语义不明确则报告最小设计问题。
