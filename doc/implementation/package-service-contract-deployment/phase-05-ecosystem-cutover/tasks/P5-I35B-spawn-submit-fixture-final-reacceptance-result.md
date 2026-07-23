# P5-I35B：Spawn Submit Fixture Final Reacceptance Result

结论：FAIL，test-runner readiness wire parity blocker。

canonical std bootstrap 1/1 PASS，fixture compile成功并启动runtime，但tests尚未执行即被
`test-runner/src/runtime_execution/wire.rs::decode_replica`拒绝：strict schema将当前Router health的
`connectionPinCount`与`connectionReleaseAckCount`视为unexpected field。hermetic root与进程均清理，candidate clean。

第三次fixture尝试已消耗。修复必须先经F46A direct与I36 cheap combined；只有新candidate且I36 PASS后，允许I35C一次
重验，理由是只有真实fixture readiness→test执行能建立此前未到达的证据，不能用decoder unit test替代。
