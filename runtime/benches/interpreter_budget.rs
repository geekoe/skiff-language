use criterion::{criterion_group, criterion_main, Criterion};

fn runtime_value_baseline(c: &mut Criterion) {
    c.bench_function("runtime_value_string_clone", |b| {
        b.iter(|| {
            let value = runtime::runtime_value::RuntimeValue::String("budget-baseline".to_string());
            std::hint::black_box(value.clone())
        })
    });
}

criterion_group!(benches, runtime_value_baseline);
criterion_main!(benches);
