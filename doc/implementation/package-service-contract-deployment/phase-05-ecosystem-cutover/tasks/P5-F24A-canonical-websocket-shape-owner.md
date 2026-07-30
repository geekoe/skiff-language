# P5-F24A：Canonical WebSocket Shape / Context Admission Owner

依赖D35 complete。独占`artifact-model/src/websocket_ingress.rs`及直接tests、必要deployment admission直接tests；不改
runtime/Router/std/compiler lowering/四对象schema。独立worktree/branch，一个clean commit。

建立唯一内部canonical shape spec，覆盖Event、ConnectRequest、ReceiveEvent、Connection、Message、ConnectResult、
ConnectionPolicy与nested Context placeholder；contract vocabulary仍只公开Event/Result两个builtin，不新增嵌套builtin名。
现有Event/Result ABI normalization必须消费同一spec。Context只允许null或同一ServiceContract内可持久化的exact nominal
ContractTypeId；CallbackInterface、foreign/missing id、cycle/interface graph在deployment/admission前fail closed。

正反tests覆盖connect/receive、accept/reject、字段集/tag、Context一致性与callback/foreign/cycle；保持四对象schema/identity
golden不变。只跑artifact/deployment精确tests、fmt/diff-check；禁止runtime smoke/full/I16/Host/stable。
