# P5-F420G1 Crate public API current oracles result

状态：`PASS`。两个 crate public API test oracle 已收敛到 current canonical policy；聚焦
验证精确 `8/8 PASS`，没有扩大到 production policy、checker、其它 test、manifest 或
lockfile。

## 1. Exact start、checkpoint 与 ancestry

- integrated repair start / tree：
  `924e8f3a246873b160ba12e2abd697b0b11c9f59` /
  `a23b9aa266a1d4dbbe655c46dfbd371acd20f4e0`；
- task-doc checkout / tree：
  `65efc72a08896549c6d5f1c6abb5b6fedb5b2a22` /
  `197f6fc0165d77a968b56578846168942a026bd8`；
- implementation checkpoint / tree：
  `e9a0c92e006842c4e7dec5aab6138a2a37c0dc93` /
  `a345db58d053e52f9e4b457e7a23c1b3df3a8a2f`。

启动与提交后分别执行 ancestry 检查：

```text
git merge-base --is-ancestor 924e8f3a... 65efc72a...
exit 0

git merge-base --is-ancestor 65efc72a... e9a0c92e...
exit 0
```

`git rev-parse 924e8f3a...^{tree}` 精确得到 batch 记录的
`a23b9aa266a1d4dbbe655c46dfbd371acd20f4e0`。task checkout 到 implementation checkpoint
的 tracked diff 只有两份授权 test。

## 2. Oracle 收敛

`crate-public-api-gate.test.mjs` 继续用
`MANAGED_CRATE_NAMES.slice(1)` 构造可用 crate 集合，再从同一
`MANAGED_CRATE_NAMES` 计算唯一缺项。错误预期通过 `escapeRegExp` 转义该派生名称后动态构造
regex，不再硬编码旧 `compiler-contract` 或当前首项。同步复用同一个可用集合后，metadata
前 fail-closed、仅一次 metadata 调用以及 explicit absence skip 的既有断言均保留。

`crate-public-api-policy.test.mjs` 的精确 current owner oracle 为：

```text
skiff-deployment
skiff-compiler-contract
skiff-compiler
```

`MANAGED_CRATE_NAMES` 与 `MANAGED_CRATE_HELP_NAMES` 仍直接从 canonical production policy
导入并接受同一有序集合；唯一性、集合一致性、冻结 snapshot/config、allow-list 与
normalization 断言语义未改变。

## 3. 聚焦验证与反向搜索

```bash
node --test \
  scripts/tests/crate-public-api-gate.test.mjs \
  scripts/tests/crate-public-api-policy.test.mjs
```

结果：

```text
tests 8
pass  8
fail  0
skipped 0
```

`git diff --check`：PASS。

旧硬编码反向搜索：

```bash
rg -n -F \
  -e 'missing.*compiler-contract' \
  -e 'only the two terminal producer owners' \
  scripts/tests/crate-public-api-gate.test.mjs \
  scripts/tests/crate-public-api-policy.test.mjs
```

结果：0 matches（`rg` exit 1）。

## 4. 边界与 clean 状态

implementation checkpoint 相对 task checkout 只修改：

```text
scripts/tests/crate-public-api-gate.test.mjs
scripts/tests/crate-public-api-policy.test.mjs
```

implementation 提交后 `git status --porcelain` 为空。result 使用独立提交且只新增本文档；
最终 result commit/tree 与 clean 状态由交付消息记录。没有运行完整 tooling、Router、
test-runner Rust suite、`run-skiff-tests`、stable 或 live；没有 merge、rebase、push。
