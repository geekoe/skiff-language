# P5-F420C Tooling fixture and format gate result

状态：`TASK_SCOPE_EXPANDED`。Isolated status fixture 的 current ownership/config receipt与
F420 test-runner rustfmt drift已在授权范围内闭合；但 missing-tar oracle 在满足全部 current
package前置后不再对应任何 production行为。要恢复原断言必须重新引入 production tar调用，违反
本任务“不得修改 production”的边界，因此 N4 未判为 PASS，F421 **未解除**。

## 1. Exact candidate 与 implementation checkpoint

- integrated start / tree：
  `273d9309c0650bad75fa08c88684359995711b91` /
  `7b860e6b026e7666c1279a3118765ddd7ff21979`；
- task checkout / tree：
  `00cdcfcbf7025aa72fdc1fccf146354d85172ede` /
  `b7b4c885dd7c61d8ae2186db8204ef24728132a2`；
- accepted F415：
  `7303af9bc9452a4d1d6e04e35b0eccb1ccacdc8d`；
- implementation checkpoint / tree：
  `5c7a1666d19a3a5c635e869a5019c344fa759f06` /
  `914dc73fba7a073c46344962eb738dbd720fe507`。

启动时 integrated start 与 accepted F415 均经 `git merge-base --is-ancestor` 验证为 HEAD
ancestor；integrated start tree精确匹配。task checkout只在 integrated start之上增加本任务文档。

## 2. 已闭合 owner

### 2.1 Isolated status failure priority

`scripts/tests/command-caller-migrations.test.mjs` 现在：

1. 在 fixture root调用 current `claimIsolatedTestWorkspace`，生成真实 marker、nonce及
   dev/ino receipt；
2. 在 owned root内创建 `instance/config.yml`；
3. 使用 `captureIsolatedTestConfig` 捕获 config owner并得到 `requireConfig` 可接受的receipt；
4. 两次 `verifyInstanceStopped` 都传同一个合法receipt。

因此第一项实际到达 fake child exit 9，并保留 stderr/stdout与无 `cause` 断言；第二项实际到达
invalid JSON并抛出 `SyntaxError`。没有伪造 dev/ino/nonce，也没有绕过 production ownership
校验。

聚焦文件最终结果为：

```text
4 tests
3 passed
1 failed
```

除 missing-tar 项外，其余三项全部通过，包括本项的两个内部 failure priority。

### 2.2 rustfmt

`test-runner/tests/package_service_contract_deployment.rs` 只接受 rustfmt 对
`package_local_abi.public_symbols["marker"]` lookup的机械单行排版：

```text
cargo fmt --all -- --check: PASS
```

测试语义、断言与数量均未改变；rustfmt没有要求修改其它文件。

## 3. Missing-tar oracle 的范围扩张

任务假定 current `package publish` 在满足 `--artifact-root` 后仍会调用外部 `tar`。实际逐层
恢复fixture前置得到：

1. 增加独立 artifact root与单一 `--artifact-root` 后，完全空的 PATH先报告
   `failed to spawn cargo: ENOENT`；
2. 在隔离 PATH只提供 current cargo/toolchain与 linker后，真实 compiler继续要求 current
   `api.yml`，且旧 `export` source语法被拒绝；
3. 再补合法 `api.yml` 与 current source语法后，同一 `package publish` **退出 0**，没有尝试
   启动 `tar`。

上述探索性修改已全部撤回，没有进入 implementation checkpoint。

全仓 production反向搜索：

```bash
rg -n \
  "failed to spawn tar|Command::new\\(\"tar\"|captureAttachedCommand\\(['\"]tar|spawn\\(['\"]tar" \
  --glob '*.{rs,mjs,ts}'
```

唯一结果是该测试自身的：

```text
scripts/tests/command-caller-migrations.test.mjs:
  assert.match(result.stderr, /failed to spawn tar: ENOENT/);
```

即 current production没有 tar command owner，也没有该错误文本。仅增加 artifact root无法让
fixture到达不存在的分支；重新引入 tar属于 production语义修改，任务明确禁止。最小后继需要先
决定：

- 删除这一已经失去 owner的 obsolete test；或
- 把它改成 current `cargo` spawn safe-outcome测试，并相应重命名，不再声称验证 tar。

这属于测试意图变更，超出 F420C 冻结的“仍实际到达 missing tar”要求。

## 4. 已执行与未执行门禁

| gate | 结果 |
| --- | --- |
| `node --test scripts/tests/command-caller-migrations.test.mjs` | 3/4 PASS；missing-tar FAIL |
| isolated status fixture | PASS；exit 9与invalid JSON均实际到达 |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check` | PASS |
| production tar owner反搜 | 0；仅测试断言1处 |

按照任务的停止条件，发现完成断言必须修改 production后立即停止，没有继续消耗或伪报以下终态
gate：

- `node scripts/verify.mjs --only tooling`；
- Node五文件组与两个 identity checker；
- `node scripts/verify.mjs --only router`；
- test-runner listing/execution；
- `node scripts/run-skiff-tests.mjs`。

F420B 已有 Router 608/608、TypeScript及其它 current-generation证据；本 checkpoint相对
integrated start未修改任何 `router/**` 或 production文件。但完整 N4要求所有本任务 gate通过，
不能仅继承 Router证据，所以 N4仍为 FAIL，F421未解除。
