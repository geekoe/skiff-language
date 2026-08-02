pub(crate) use super::*;
use skiff_runtime_model::runtime_value::RuntimeValue;

#[test]
fn rollback_checkpoint_keeps_only_slots_occupied_at_transaction_entry() {
    let layout = RuntimeSlotLayout {
        count: 2,
        bindings: vec![
            slot_store::RuntimeSlotBinding {
                slot: 0,
                name: "outer".to_string(),
                kind: "local".to_string(),
                scope: None,
            },
            slot_store::RuntimeSlotBinding {
                slot: 1,
                name: "transaction-local".to_string(),
                kind: "local".to_string(),
                scope: None,
            },
        ],
        self_slot: None,
        parameter_slots: Default::default(),
    };
    let mut env = Env::with_slot_layout(&layout);
    env.declare_binding("outer", Some(0), RuntimeValue::Null)
        .expect("outer declaration");
    let checkpoint = env.rollback_checkpoint();
    env.assign_binding("outer", Some(0), RuntimeValue::Number(7.0))
        .expect("outer assignment");
    env.declare_binding("transaction-local", Some(1), RuntimeValue::Number(9.0))
        .expect("transaction-local declaration");

    let roots = env
        .rollback_root_carriers(&checkpoint)
        .expect("rollback root projection");
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].0, 0);
    let candidate = env
        .rebased_for_rollback(&checkpoint, &roots)
        .expect("rollback Env candidate");
    assert_eq!(
        candidate.get_slot(0).expect("outer survives").into_value(),
        RuntimeValue::Number(7.0)
    );
    assert!(
        candidate.get_slot(1).is_err(),
        "a slot that was empty at transaction entry must not escape rollback"
    );
}
mod detached;
