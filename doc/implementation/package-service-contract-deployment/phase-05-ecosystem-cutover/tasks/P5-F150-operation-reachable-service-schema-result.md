# P5-F150：Operation-reachable Service Schema 结果

结论：PASS

- 父节点：`P5-D84-instance-interface-service-schema-audit-result.md`
- commit `51afe82` 已合入 Phase 5 integration。
- ServiceContract schema从 operation/callback seeds lazy closure；unreachable public interface不进入，reachable invalid
  callback仍 fail closed。
- projection 7/7、compiler-contract crate 19/19 PASS。

