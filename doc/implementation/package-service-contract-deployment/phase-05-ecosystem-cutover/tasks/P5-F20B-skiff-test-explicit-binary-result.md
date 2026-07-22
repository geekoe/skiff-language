# P5-F20B：`skiff test` Explicit Binary Result

`F20B PASS`

开发提交`1308ab58d537f13b16d54abdde788b01e1c83b3d`，parent
`cd6342733113713bb092616d51dd6d862abbcb61`，tree `d5b45e52135887385446fd5b088b5ef9befc2d8b`，lock不变。
只修改`scripts/skiff.mjs` test caller与direct argv test。

production argv精确为`cargo run --locked --quiet --manifest-path <Cargo.toml> --bin skiff-test-runner -- <runner argv>`；
absolute/relative root各exact一次selector，hostile env不能改变，原有artifact/platform/base/live/strict顺序不变。Node 5/5、
两个node check与diff check PASS；未加default-run，未触碰runtime-live/encrypted/T06、manifest/lock，extra-review无blocker。
