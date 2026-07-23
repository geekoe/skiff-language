# P5-I07A：skiff-packages Runtime Reacceptance

I07因owner漏传`SKIFF_ROOT`而无有效结论，环境已清理。全新owner重复其exact detached validation worktree与ignored
dependency link方案，但四条命令必须全部显式：

```bash
SKIFF_ROOT="$P5_VALIDATION_SKIFF_ROOT" npm run test:aliyunoss
SKIFF_ROOT="$P5_VALIDATION_SKIFF_ROOT" npm run test:http-session
SKIFF_ROOT="$P5_VALIDATION_SKIFF_ROOT" npm run test:openai
SKIFF_ROOT="$P5_VALIDATION_SKIFF_ROOT" npm run test:track
git diff --check
```

候选packages `ecb7485286fd4df6f2fed78022c75a2ad9c3cc36`，Skiff R02
`8ecf41ce9581714b8c72617d4d0c612982dc6899`。每条一次，结束完整清理；禁止stable/修改/retry。
