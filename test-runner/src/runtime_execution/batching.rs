use std::path::PathBuf;

use crate::test_discovery::TestServiceCase;

pub(super) const MAX_NON_LIVE_CASES_PER_ACTIVATION: usize = 16;

pub(super) fn max_non_live_cases_per_activation() -> usize {
    std::env::var("SKIFF_TEST_MAX_CASES_PER_ACTIVATION")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(MAX_NON_LIVE_CASES_PER_ACTIVATION)
}

pub(super) fn partition_cases(
    cases: Vec<TestServiceCase>,
    live: bool,
) -> Vec<Vec<TestServiceCase>> {
    if live {
        vec![cases]
    } else {
        partition_non_live_cases(cases)
    }
}

pub(super) fn partition_non_live_cases(cases: Vec<TestServiceCase>) -> Vec<Vec<TestServiceCase>> {
    let mut batches = Vec::new();
    let mut current_batch = Vec::new();
    let mut file_cases = Vec::new();
    let mut current_path: Option<PathBuf> = None;

    for case in cases {
        if current_path
            .as_ref()
            .is_some_and(|path| path != &case.relative_path)
        {
            pack_file_cases(&mut batches, &mut current_batch, file_cases);
            file_cases = Vec::new();
        }
        current_path = Some(case.relative_path.clone());
        file_cases.push(case);
    }
    pack_file_cases(&mut batches, &mut current_batch, file_cases);
    if !current_batch.is_empty() {
        batches.push(current_batch);
    }
    batches
}

fn pack_file_cases(
    batches: &mut Vec<Vec<TestServiceCase>>,
    current_batch: &mut Vec<TestServiceCase>,
    file_cases: Vec<TestServiceCase>,
) {
    let cap = max_non_live_cases_per_activation();
    if file_cases.is_empty() {
        return;
    }
    if file_cases.len() <= cap {
        if current_batch.len() + file_cases.len() > cap {
            batches.push(std::mem::take(current_batch));
        }
        current_batch.extend(file_cases);
        return;
    }

    if !current_batch.is_empty() {
        batches.push(std::mem::take(current_batch));
    }
    let mut remaining = file_cases.into_iter();
    loop {
        let chunk = remaining
            .by_ref()
            .take(cap)
            .collect::<Vec<_>>();
        if chunk.is_empty() {
            return;
        }
        if chunk.len() == cap {
            batches.push(chunk);
        } else {
            *current_batch = chunk;
            return;
        }
    }
}

pub(super) fn batch_execution_scope(run_scope: &str, batch_index: usize) -> String {
    format!("{run_scope}-batch-{batch_index}")
}
