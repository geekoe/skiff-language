# P5-F420G3 HTTP-only tooling smoke closure result

状态：`TASK_SCOPE_EXPANDED`。预检发现 current package-test service identity 的必改 oracle
与 I02 focused gate 中的未授权 fixture 互相冲突。本节点在任何 implementation 写入前立即停止；
没有 implementation commit，也不宣称 HTTP-only tooling smoke 已闭合。

## 1. Exact start 与停止点

- integrated repair start / tree：
  `924e8f3a246873b160ba12e2abd697b0b11c9f59` /
  `a23b9aa266a1d4dbbe655c46dfbd371acd20f4e0`；
- task checkout / tree：
  `65efc72a08896549c6d5f1c6abb5b6fedb5b2a22` /
  `197f6fc0165d77a968b56578846168942a026bd8`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`。

`git merge-base --is-ancestor <integrated-repair-start> HEAD` 返回 0。task checkout 相对
integrated repair start 只增加 F420G batch 与五个 leaf task 文档，没有 executable diff。
最终 result-only commit 由交付消息记录；它同样不改变 executable candidate。

## 2. 精确范围冲突

current Rust producer
`test-runner/src/package_test_assembly.rs:187` 对每个 package test case 生成：

```text
test.skiff/package/{safe-coordinate}/case-{index}
```

因此 G3 必须把
`scripts/lib/package-service-ecosystem-smoke-oracle.mjs:78` 与 shared fixture 收敛为
`test.skiff/package/<package-id>/case-0`。这也是任务合同第 1 项明确要求的 fail-closed
current identity。

但是 required focused gate
`scripts/tests/package-service-i02-combined.test.mjs:417-418` 在把 shared receipt 改造成 I02
fixture 时，又显式覆盖为无 case suffix 的旧值：

```js
fixture.candidate.entrypoints[0].deployment.serviceId =
  `test.skiff/package/${packageId}`;
```

`runPackageServiceI02Combined` 会用
`packageId = test.skiff/package-service-i02-spawn-submit` 调用同一个
`readPackageServiceFixtureReceipt`。一旦按授权修正 shared oracle，phase 33 会在 authoring receipt
解析处得到：

```text
actual:   test.skiff/package/test.skiff/package-service-i02-spawn-submit
expected: test.skiff/package/test.skiff/package-service-i02-spawn-submit/case-0
```

也就是说，不修改该 I02 test fixture 就无法同时满足：

1. Rust producer 的 exact current service id；
2. shared oracle 的 fail-closed `/case-0` 要求；
3. 任务指定的 `package-service-i02-combined.test.mjs` focused PASS。

该 test 不在 G3 的允许写入集合中，任务还明确禁止修改“其它 tests”。通过 production
normalization、兼容读取或放宽 oracle 绕过都会违反 HTTP-only/current-only 收敛要求，故没有采用。

## 3. 最小 successor 范围

后继任务需要把
`scripts/tests/package-service-i02-combined.test.mjs`
加入允许写入，并只把上述 fixture service id 更新为：

```js
`test.skiff/package/${packageId}/case-0`
```

这是 current test fixture 修正，不要求修改 I02 production owner、Router、Rust producer、
test-runner、fixture source、manifest、lockfile 或 verify plan。完成这一合同修订后，原 G3 的
HTTP utility 迁移、旧 Assembly-WebSocket owner 删除、反向搜索与 focused matrix 才能继续。

## 4. 停止证据与边界

预检命令：

```bash
rg -n \
  "test\\.skiff/package|case-\\{|case-" \
  test-runner/src/package_test_assembly.rs \
  scripts/tests/package-service-i02-combined.test.mjs \
  scripts/lib/package-service-ecosystem-smoke-oracle.mjs

git status --porcelain=v1
git diff --check
```

结果分别精确定位 Rust producer `:187`、shared oracle `:78` 与 I02 fixture `:417-418`；
停止前 `git status --porcelain=v1` 为空且 `git diff --check` PASS。因为 executable tree 从未
改变，没有运行会重复既有 candidate 证据的 focused tests，也没有删除旧 generation phases。

本节点没有修改任何 production/test 文件，没有 implementation commit，没有运行完整 tooling、
Router、test-runner Rust suite、`run-skiff-tests`、stable 或 live；没有 merge、rebase、push 或
操作 instance/watch registry。
