# P5-F420G2 Dev-sync RuntimeAssembly v2 fixture result

状态：`PASS`。Dev-sync fake compiler receipt 已收敛到 canonical RuntimeAssembly v2；
聚焦测试从 4/5 恢复为 5/5，且目标测试文件中不再存在 RuntimeAssembly v1 identity。

## 1. Exact start 与 implementation checkpoint

- batch integrated start / tree：
  `924e8f3a246873b160ba12e2abd697b0b11c9f59` /
  `a23b9aa266a1d4dbbe655c46dfbd371acd20f4e0`；
- task-doc checkout / tree：
  `65efc72a08896549c6d5f1c6abb5b6fedb5b2a22` /
  `197f6fc0165d77a968b56578846168942a026bd8`；
- implementation commit / tree：
  `2a02ce30d7748d2cfba86c776761e4009559b1f8` /
  `0f6d34abb02d202b551ece9de9f9f48f2263d958`。

Batch integrated start 是 task-doc checkout 的直接父提交，task-doc checkout 是 implementation
commit 的直接父提交；实现从 batch 指定的 exact start/tree 及其任务文档 checkout 启动。

## 2. 基线与实现

在 task-doc checkout 上执行：

```bash
node --test scripts/tests/package-service-dev-sync.test.mjs
```

得到 4/5；唯一失败为：

```text
dev sync has one package phase and consumes generated service receipts before assembly
Error: assembly activation requires an exact RuntimeAssembly reference
```

实现只修改 `scripts/tests/package-service-dev-sync.test.mjs` 的共享 fake assembly identity：

```text
skiff-runtime-assembly-v1:sha256:...
→
skiff-runtime-assembly-v2:sha256:...
```

没有修改 production dev-sync / activation、测试声明或断言，也没有增加 v1 compatibility、
dual-read、fallback 或其它 authoring inference。

## 3. 聚焦验证

| 命令 | 结果 |
| --- | --- |
| `node --test scripts/tests/package-service-dev-sync.test.mjs` | PASS，5/5 |
| `rg -n "skiff-runtime-assembly-v1" scripts/tests/package-service-dev-sync.test.mjs` | 0 个匹配；`rg` 以预期的无匹配状态 1 退出 |
| `rg -n "skiff-runtime-assembly-v2" scripts/tests/package-service-dev-sync.test.mjs` | 1 个匹配，位于 canonical fake identity |
| `git diff --check` | PASS |

## 4. 范围闭合

`git diff --name-status 65efc72a..2a02ce30` 只有：

```text
M scripts/tests/package-service-dev-sync.test.mjs
```

实现 diff 为一行 v1 → v2 identity 替换；其余四项测试的声明、执行路径与断言均未改动。
按任务边界未运行完整 tooling、Router、test-runner Rust suite、`run-skiff-tests`、stable 或 live，
也未执行 merge、rebase 或 push。
