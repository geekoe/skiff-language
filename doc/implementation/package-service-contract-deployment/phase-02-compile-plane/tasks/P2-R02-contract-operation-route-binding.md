# P2-R02：Contract Operation Route Binding

状态：deferred；不属于 Phase 02，不执行本文旧实现。

本能力必须由 Phase 03 的 `ServiceDeployment` / `RuntimeAssembly` binding 与 Phase 04 的
`InProcessBoundary` dispatcher 共同落地：assembly 显式把 `ContractOperationId` 绑定到已校验的
package callable，不借用旧 runtime selector、`ServiceUnit` 或临时路由表。

Phase 02 最终 tree 不保留本任务产生的 runtime host 改动。Phase 03 细化时重新建立任务和验收，
本文件仅记录 disposition，不是可执行指令。
