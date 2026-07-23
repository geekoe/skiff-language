# P5-I27：Fixture Pipeline Cheap Combined

全新只读integration Agent在F27A/B/C合流的exact clean candidate执行一次；不编辑/提交，不启动Router/runtime/activation，
不运行top-level smoke/full/I16/Host/stable。

组合一次运行：compiler official std authoring/writer正反；test-runner seed CAS/idempotency与missing-std负例；D38C exact
ecosystem regression；F27C receipt/readiness/F26A diagnostic Node tests；compiler/runtime DAG、typecheck/diff-check。若exact
regression PASS，再按真实bootstrap顺序执行一次不启服务的fixture Cargo并验证typed receipt/records。每组非零、fail-fast，
PASS只解锁R29。
