# P5-F46B：Fixture Cargo Causal Diagnostic Result

结论：COMPLETE。

- task commit：`40976cabaf7fef3ccb4ace7afbfd81ca2cf6b556`
- integration commit：`00649e5b459913c957c28a437368bac8a9e48acf`
- tree：`47b392ac42b8ec7563151ca4b5b35a107ef23a3f`

bounded diagnostic现在优先process summary、Caused by与Cargo/compiler error，最多3条；脱敏、512/1536 bytes、hash、
bytes、line SHA与omitted count保持。direct 11/11 PASS。
