# P5-F20A：Gate Bounded Diagnostic Retention Result

`F20A PASS`

开发提交`d95999fce4657d4fe4e95484e0e0fc90364a3fbb`，parent
`cd6342733113713bb092616d51dd6d862abbcb61`，tree `ade742311c1395193fe7bf23c6120bf18895ae2a`，lock不变。
只修改合同四个gate evidence/test路径。

ledger升v5；Host nonzero保留startup/std/host-prepare/host-runner/unknown phase、subject、stdout/stderr byte counts与
512-byte bounded first diagnostic。integration/A/B/task/temp/home路径、secret与HTTP body脱敏；原始行/全输出只保留SHA。
validator从原始outcome重算，primary-first及原始hash不变。filtered command-double 17/17、三个node check、diff check PASS；
未运行真实Cargo/I16/H18/Host/full。诊断parser已抽为单一189行child owner，extra-review无blocker。
