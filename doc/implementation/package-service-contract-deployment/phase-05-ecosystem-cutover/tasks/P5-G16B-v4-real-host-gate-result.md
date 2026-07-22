# P5-G16B：V4 Real Host Gate Result

`G16 FAIL`

本周期第二次full-mode调用锚定`f82282c2dfde25a2f2c2505b536ee2f9a3fc73cb` / tree
`1055fc2a49962d3657ee3fab84712162c872de56` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`，exit 1且未重试。v4 combined ledger SHA-256为
`5e9613d7222abc8943bb8c64d7ae6bcedbbc3c926a7a730f53469e613977d90b`，digest为
`237676b55a4eaa171cd63fa700bc39442f0b633c3505ec5f594d7aff76f4b062`。

artifact gate真实PASS：四个targeted crates全Fresh；21个artifact中只有
`skiff-{test-runner,package-service-smoke-fixture}.d`两个顶层dep-info发生各369次exact A→B root materialization，
stable binary/rlib/hashed dep-info无变化，`changed=2/allowed=2/disallowed=0`。

真实`run-skiff-tests.mjs`首次启动，code 1/signal null，`fullProbeRuns:1`；没有result/PASS lines，std 11/11、Host 1/1与
final value均未建立。Gate ledger digest为`beb3b32dea250627769bba013c790a679236bb315f2e3ae750469e63ca05d64c`。
v4只保存stdout/stderr SHA而未保存有界首错，故无法从现有证据区分startup、std assembly/activation/request或Host prepare。

cleanup全部PASS：A/B/Git admin/task root、22个PID、46234–46236端口与lease均ABSENT，foreign preserved、errors为空；
前后candidate/combined ledger不变。本周期full-mode累计2、真实Host attempt累计1、完整positive累计0。第三次前必须D26
审计、批量修复、fresh combined/验收与独立preflight；不得直接重试。
