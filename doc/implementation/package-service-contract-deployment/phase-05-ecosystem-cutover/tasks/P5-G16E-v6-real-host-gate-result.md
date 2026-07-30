# P5-G16E：V6 Real Host Gate Result

`G16E PASS`

- candidate：`411f9b63114db6699a747df986698c570985299b`
- tree：`aab80bde42967f2f478eed66388002829172a8ae`
- `Cargo.lock` blob：`f3ce5457138c58aec4c84abda431afa96013e3fd`
- 唯一full调用1次、无重试，exit 0，耗时72.77秒；这是本收敛周期第2次、历史full #5和预算上限。

v6/full顶层与primary均PASS。artifact四个targeted crate全部Fresh，A→B仅两个允许的top-level `.d`
materialization，`changed=2/allowed=2/disallowed=0`。owned-B offline install与B-local `tsx --version`各一次且
code 0。真实Host child只启动一次且code 0，结果依次为std `11/11`、Host `1/1`；唯一actual为
`PASS main.__test::provider observes helper mutation`，最终值为`provider-observed-helper-mutated`，fixture assertion精确。

full raw JSON为142109 bytes，SHA-256
`a43a31bc2c5be83e8dd58e0d27677f4556a5a65036ceda202da1097217531cc7`，内部ledger digest
`75504c9434a4fd7b752a8106dc6d25cca0b832e1095d396390d3c9155c4f1ea0`经独立复算一致；原始会话证据nonce为
`c73cc8465225b53477931496b0e54b7d`，没有独立落盘文件。combined ledger前后保持bit-identical：文件SHA
`1cf4dbd25ab5c7ea4701b84245f077b3739691e746aedc17d22a8b03e9d3f364`，内部digest
`a8958ba6d8bf4456c520f269eefd710245fc62fb0876d3045522150d3eb49109`。

24个进程、端口46087–46089、A/B worktree、Git登记、task/shared target与dependencies均清理，foreign preserved、
errors 0、`stableOperations: 0`。候选身份和tracked状态前后不变，唯一untracked仍为I16 combined ledger。
该PASS只解锁同一候选的R23，不单独给F04或Phase 5 verdict。
