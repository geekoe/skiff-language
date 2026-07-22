# P5-P20A：Official Std Exact Probe Result

`P20A PASS`

锚定`f82282c2dfde25a2f2c2505b536ee2f9a3fc73cb` / tree
`1055fc2a49962d3657ee3fab84712162c872de56` / lock
`f3ce5457138c58aec4c84abda431afa96013e3fd`。唯一命令精确执行一次：

```bash
cargo test --locked -p skiff-test-runner --test package_service_contract_deployment \
  official_platform_package_is_compiled_as_the_selected_source_root -- --exact --test-threads=1
```

结果`1 passed / 0 failed / 0 ignored / 12 filtered out`。覆盖current official std compile、11 case discovery、overlay
projection、contract/deployment/runtime assembly与canonical publish，并拒绝非canonical root冒充reserved ID。未启动isolated
runtime/source suite/Host；前后candidate与v4 ledger不变、tracked clean。
