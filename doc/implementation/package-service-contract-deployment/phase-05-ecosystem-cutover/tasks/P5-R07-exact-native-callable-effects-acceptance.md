# P5-R07：Exact Native Callable Effects Acceptance

未参与F07实现的独立只读Agent。输入为F07 exact clean integration commit/tree、D08/F07合同、F06/R06 checkpoint
与聚焦证据；不得编辑、提交、修复或给F04/R02 verdict。

必验：

- shared semantics以exact binding key稀疏登记，缺项默认Unknown；只含四个审计过的context-free string native；
- compiler只有exact NativeFunction target命中descriptor，custom/raw/dynamic/crypto/capability native保持fail closed；
- effects/return provenance与immutable scalar语义一致，无caller alias/mutation/suspend/same-heap放宽；
- lowering/FileIR仍使用既有native invocation与binding key，无artifact schema变化或第二identity owner；
- runtime validator拒绝unknown/duplicate/signature mismatch/context capability/missing handler descriptor；handler未改；
- canonical truncate std package projection正例真实通过，恶意负例非fixture symbol特判；`extra-review`无重复registry。

第一行只给`R07 PASS`或`R07 FAIL`。PASS只解锁F04恢复完整source suite/接收；FAIL给最小native反例与owner。
