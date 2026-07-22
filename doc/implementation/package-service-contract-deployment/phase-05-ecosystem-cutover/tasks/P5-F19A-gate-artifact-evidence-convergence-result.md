# P5-F19A：Gate Artifact / Evidence Convergence Result

`F19A PASS`

开发提交`4ef77bc63d2da3a360ef22d954b65c2e6352ced8`，parent `67daaa18bd634ea83d07a9b6440a57de51265b45`，
tree `44840f4e336b90797b1ff6f918a6655edf074785`，lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。只修改合同exclusive gate evidence/test路径。

- ledger升级v4；validator重算artifact evidence，旧v3显式失效。
- combined保留全artifact hash/mtime严格相等；full只允许exact A-root→B-root顶层dep-info materialization，
  binary/rlib/hashed dep-info、缺Fresh或Fresh+Dirty/Compiling均fail closed。
- ledger在断言前保存before/outcome/after/structured diff；失败保留exact path/classification/hash/mtime/size。
- Host发起前记attempt 1并保留code/signal/output digest；exact PASS line + 11/11 + 1/1才产生finalValue。
- command-double 27/27、四个node check及diff check PASS；旧I16 A/B/A corpus离线分类通过。未运行真实Cargo/I16/Host/full。

Extra-review未发现第二comparator或重复owner；538行evidence module函数均小于100行，职责为单一canonical evaluator。
