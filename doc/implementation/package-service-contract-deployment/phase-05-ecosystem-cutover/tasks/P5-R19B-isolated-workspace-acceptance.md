# P5-R19B：Isolated Workspace Ownership Acceptance

使用未参与F19B、D25、I16或其它验收的全新独立只读Agent。输入为F19B commit/result、同一final candidate/lock与v4
I16 PASS bundle；前后tracked clean且唯一untracked为该ledger。第一行只给`R19B PASS/FAIL`。

必验：workspace/config owner为单一nonce+marker+dev/ino+realpath receipt；down/status/remove每次复验，foreign replacement/
symlink/missing/corrupt均不调用foreign config或递归删除。primary-first/all-settled关闭owned child/ports/lease，startup partial与
normal/signal路径共享owner；无force、sibling enumeration或第二cleanup。port lease未实证TOCTOU只作为明确残余风险。

唯一抽查：

```bash
node --test --test-name-pattern 'isolated runtime carries|foreign workspace replacements|primary failure stays first' \
  scripts/tests/isolated-test-runtime-workspace-cases.mjs
```

必须报告matched>0与pass/fail/skipped；不运行真实runtime、全部开发矩阵、I16/H18/Host/full/stable，不修改/提交。
运行extra-review并回报identity/ledger、foreign preservation、blocker与残余风险。
