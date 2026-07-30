# P5-D84：Instance Interface Service Schema 审计结果

结论：`READY_TO_IMPLEMENT`

## 父节点链

- `P5-D84-instance-interface-service-schema-audit.md`
- 向上追溯到 F145 result、C2 batch 和唯一权威设计。

## 根因与 owner

- source compiler 正确把 public instance interface methods 投影成 executable operations。
- `compiler/contract::project_boundary_schema` 在计算 operation reachability 前 eager materialize全部 public types/interfaces，
  并把 interface 一律当 callback schema，导致不相关 `std.http.HttpRequest` 被判 non-materializable。
- ServiceContract 只拥有 operation/callback 可达 schema。公开 Package API/interface declaration 不因公开或承载
  instance methods自动进入 ServiceContract schema。
- canonical owner 是 `compiler/contract/src/projection.rs`：先建索引，再从 operation/callback ContractTypeId seeds lazy
  project closure；reachable invalid interface仍必须 fail closed。

删除 consumer public interface会掩盖共享 bug，不是允许的修复。

