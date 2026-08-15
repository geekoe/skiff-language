use skiff_artifact_model::{
    OperationCallableKind, PackageCallableId, PackageCallableLinkFact, PackageLocalAbiSymbol,
};

use super::super::canonical::{
    canonical_binding_public_callable, canonical_implementation_callable, CanonicalCallableError,
};
use super::ProjectionFixture;

fn add_canonical_callable(fixture: &mut ProjectionFixture) -> PackageCallableId {
    let public_callable = fixture.callable_id.clone();
    let signature = match fixture
        .implementation
        .package_local_abi
        .public_symbols
        .get("handle")
        .unwrap()
    {
        PackageLocalAbiSymbol::Callable { signature, .. } => signature.clone(),
        _ => unreachable!("fixture public callable must be a Callable symbol"),
    };
    let facts = fixture
        .implementation
        .callable_semantic_facts
        .get(&public_callable)
        .unwrap()
        .clone();
    let mut target = fixture.implementation.callable_links[&public_callable]
        .target
        .clone();
    let canonical_callable =
        PackageCallableId::new("pkg-callable:example.provider:top-level:provider.main.handle");
    target.callable_abi_id = canonical_callable.to_string();
    target.callable_kind = OperationCallableKind::InternalFunction;
    fixture
        .implementation
        .package_local_abi
        .implementation_symbols
        .insert(
            "provider.main.handle".to_string(),
            PackageLocalAbiSymbol::Callable {
                callable_id: canonical_callable.clone(),
                signature,
            },
        );
    fixture.implementation.callable_links.insert(
        canonical_callable.clone(),
        PackageCallableLinkFact {
            callable_id: canonical_callable.clone(),
            target,
        },
    );
    fixture
        .implementation
        .callable_semantic_facts
        .insert(canonical_callable.clone(), facts);
    fixture.refresh_implementation_ref();
    canonical_callable
}

fn add_second_public_alias(fixture: &mut ProjectionFixture) -> PackageCallableId {
    let public_callable = fixture.callable_id.clone();
    let signature = match fixture
        .implementation
        .package_local_abi
        .public_symbols
        .get("handle")
        .unwrap()
    {
        PackageLocalAbiSymbol::Callable { signature, .. } => signature.clone(),
        _ => unreachable!("fixture public callable must be a Callable symbol"),
    };
    let alias = PackageCallableId::new("pkg-callable:example.provider:handleAlias");
    let mut target = fixture.implementation.callable_links[&public_callable]
        .target
        .clone();
    target.callable_abi_id = alias.to_string();
    fixture
        .implementation
        .package_local_abi
        .public_symbols
        .insert(
            "handleAlias".to_string(),
            PackageLocalAbiSymbol::Callable {
                callable_id: alias.clone(),
                signature,
            },
        );
    fixture.implementation.callable_links.insert(
        alias.clone(),
        PackageCallableLinkFact {
            callable_id: alias.clone(),
            target,
        },
    );
    fixture.implementation.callable_semantic_facts.insert(
        alias.clone(),
        fixture.implementation.callable_semantic_facts[&public_callable].clone(),
    );
    fixture.implementation.boundary_projections.insert(
        alias.clone(),
        fixture.implementation.boundary_projections[&public_callable].clone(),
    );
    fixture.refresh_implementation_ref();
    alias
}

#[test]
fn public_alias_resolves_to_canonical_manifest_callable() {
    let mut fixture = ProjectionFixture::new();
    let canonical = add_canonical_callable(&mut fixture);

    assert_eq!(
        canonical_implementation_callable(&fixture.implementation, &fixture.callable_id).unwrap(),
        canonical
    );
    assert_eq!(
        canonical_binding_public_callable(&fixture.implementation, "echo", &canonical).unwrap(),
        fixture.callable_id
    );
    assert_eq!(
        canonical_binding_public_callable(&fixture.implementation, "handle", &canonical).unwrap(),
        fixture.callable_id
    );
}

#[test]
fn missing_canonical_implementation_fails_closed() {
    let fixture = ProjectionFixture::new();
    let error = canonical_implementation_callable(&fixture.implementation, &fixture.callable_id)
        .unwrap_err();
    assert!(matches!(
        error,
        CanonicalCallableError::MissingCanonicalImplementation { .. }
    ));
}

#[test]
fn ambiguous_canonical_owners_fail_closed() {
    let mut fixture = ProjectionFixture::new();
    let first = add_canonical_callable(&mut fixture);
    let mut duplicate_target = fixture.implementation.callable_links[&first].target.clone();
    duplicate_target.callable_abi_id =
        "pkg-callable:example.provider:top-level:provider.main.duplicate".to_string();
    let duplicate =
        PackageCallableId::new("pkg-callable:example.provider:top-level:provider.main.duplicate");
    fixture
        .implementation
        .package_local_abi
        .implementation_symbols
        .insert(
            "provider.main.duplicate".to_string(),
            PackageLocalAbiSymbol::Callable {
                callable_id: duplicate.clone(),
                signature: match fixture
                    .implementation
                    .package_local_abi
                    .implementation_symbols
                    .get("provider.main.handle")
                    .unwrap()
                {
                    PackageLocalAbiSymbol::Callable { signature, .. } => signature.clone(),
                    _ => unreachable!(),
                },
            },
        );
    fixture.implementation.callable_links.insert(
        duplicate.clone(),
        PackageCallableLinkFact {
            callable_id: duplicate.clone(),
            target: duplicate_target,
        },
    );
    fixture.implementation.callable_semantic_facts.insert(
        duplicate.clone(),
        fixture.implementation.callable_semantic_facts[&first].clone(),
    );

    let error = canonical_implementation_callable(&fixture.implementation, &fixture.callable_id)
        .unwrap_err();
    let CanonicalCallableError::AmbiguousCanonicalImplementation {
        first: first_id,
        second: second_id,
        ..
    } = error
    else {
        panic!("expected ambiguous canonical owners");
    };
    assert!(
        [first_id, second_id]
            .iter()
            .all(|id| *id == first || *id == duplicate),
        "ambiguous owner set must contain both canonical callables"
    );
}

#[test]
fn drifted_canonical_link_fails_closed() {
    let mut fixture = ProjectionFixture::new();
    let canonical = add_canonical_callable(&mut fixture);
    fixture
        .implementation
        .callable_links
        .get_mut(&canonical)
        .unwrap()
        .target
        .callable_abi_id = "pkg-callable:example.provider:forged".to_string();

    let error = canonical_implementation_callable(&fixture.implementation, &fixture.callable_id)
        .unwrap_err();
    assert!(matches!(
        error,
        CanonicalCallableError::InvalidCallableLink { .. }
    ));
}

#[test]
fn ambiguous_public_alias_without_stable_key_fails_closed() {
    let mut fixture = ProjectionFixture::new();
    let canonical = add_canonical_callable(&mut fixture);
    add_second_public_alias(&mut fixture);

    let error =
        canonical_binding_public_callable(&fixture.implementation, "echo", &canonical).unwrap_err();
    assert!(matches!(
        error,
        CanonicalCallableError::AmbiguousPublicCallable { .. }
    ));
    assert_eq!(
        canonical_binding_public_callable(&fixture.implementation, "handle", &canonical).unwrap(),
        fixture.callable_id
    );
}
