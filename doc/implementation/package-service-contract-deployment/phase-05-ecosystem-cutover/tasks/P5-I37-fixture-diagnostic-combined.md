# P5-I37：Fixture Diagnostic Combined

DAG节点I37，依赖F46B合流。全新只读owner各运行一次：

```bash
node --test \
  scripts/tests/package-service-ecosystem-smoke-diagnostic.test.mjs \
  scripts/tests/package-service-i02-combined.test.mjs
git diff --check
```

必须确认长warning前缀/后缀中的terminal error保留、最多3条、脱敏/限长/hash/bytes/omitted count及I02调用接线。
禁止真实Cargo/fixture/I02/R05、编辑、提交、instance/stable/full gate。PASS只解除I02B一次真实combined。
