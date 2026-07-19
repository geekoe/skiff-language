# P2-R07：Service-test Local Entrypoint

状态：deferred；不属于 Phase 02，不执行本文旧实现。

保留的语义判断是：test case 是 package-local test entrypoint，不是公开 `ServiceContract`
operation。具体 assembly、activation 与 dispatch 必须等 Phase 03/04 的终态 owner 落地后重新拆解，
不能引入 `ServiceTestAssembly` 临时 sidecar、`ServiceUnit` 壳或 test-only contract wrapper。

Phase 02 最终 tree 只保留 canonical package compiler/test fixture；旧 integration 中的 runtime、
test-runner 与 router 改动不进入新 branch。本文件仅记录 disposition，不是可执行指令。
