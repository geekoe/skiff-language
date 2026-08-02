# Router Rust Migration Batch 1 Test Ledger

按权威设计 `doc/implementation/router-rust-migration-plan.md` §9：
每删除一个 TS test，ledger 标记 `retired` / `shared owner` / Rust replacement /
black-box replacement；不能以类型系统代替 observable test。

本 ledger 只记录 C0-control 节点在 baseline `main@9e492fa7` 上对 router TS tests 的处置。

| 测试文件（baseline） | 处置 | 理由 | 替代 / 新 owner |
| --- | --- | --- | --- |
| `router/tests/artifact-reload.test.ts` | retired（文件删除） | `/__skiff/reload-artifacts` 端点按 C0 设计删除，canonical control 契约统一为 `/__skiff/activate-assembly`；reload 语义用例不再有可观察行为 | reload 相关用例由删除行为本身取代；保留用例（runtime control broadcast、prune-runtimes）re-owned 到 `router/tests/router-control-plane.test.ts`，并新增 stale 路径 404 负例 |
| `router/tests/loop-risk-health.test.ts` | shared owner / re-owned | `?detail=loop-risk` 投影从 legacy `RouterControlPlane`（仅测试使用）移入 production `AssemblyControlPlane` canonical owner | 文件保留，fixture 改为 `AssemblyControlPlane` + `AssemblyRuntimeRegistry`；reconnect 语义更新为 canonical replica 替换（旧会话记录被新 replica 记录替换，不再保留双 session） |

未删除、未改动的 router tests 不在本 ledger 记录。
