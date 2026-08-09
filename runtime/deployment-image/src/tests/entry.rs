use std::sync::Arc;

use crate::{
    DeploymentImage, DeploymentOwnerIdentity, DeploymentProgramEntry, DeploymentProgramFacts,
    PinnedDeploymentEntry, PinnedDeploymentEntryError, ServiceDependencySlot,
};

use super::owner;

#[derive(Debug, PartialEq, Eq)]
struct NonCloneProgram {
    owner: DeploymentOwnerIdentity,
    dependency_slots: Box<[ServiceDependencySlot]>,
    label: &'static str,
}

impl DeploymentProgramFacts for NonCloneProgram {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    fn dependency_slots(&self) -> &[ServiceDependencySlot] {
        &self.dependency_slots
    }
}

#[derive(Debug)]
struct FakeEntry {
    owner: DeploymentOwnerIdentity,
    program: Arc<NonCloneProgram>,
    name: &'static str,
}

impl DeploymentProgramEntry<NonCloneProgram> for FakeEntry {
    fn owner(&self) -> &DeploymentOwnerIdentity {
        &self.owner
    }

    fn program(&self) -> &Arc<NonCloneProgram> {
        &self.program
    }
}

#[test]
fn entry_pins_the_same_non_clone_program_and_exact_owner() {
    let exact_owner = owner("build:entry");
    let program = non_clone_program(exact_owner.clone(), "verified");
    let image = Arc::new(
        DeploymentImage::try_new(Arc::clone(&program)).expect("empty dependency set is valid"),
    );
    let pinned = PinnedDeploymentEntry::try_new(
        Arc::clone(&image),
        FakeEntry {
            owner: exact_owner.clone(),
            program: Arc::clone(&program),
            name: "gateway-entry",
        },
    )
    .expect("entry and image share the same program allocation");

    let object_safe_entry: &dyn DeploymentProgramEntry<NonCloneProgram> = pinned.entry();
    assert_eq!(object_safe_entry.owner(), &exact_owner);
    assert!(Arc::ptr_eq(object_safe_entry.program(), &program));
    assert!(Arc::ptr_eq(pinned.image(), &image));
    assert_eq!(pinned.owner(), image.owner());
    assert_eq!(pinned.entry().name, "gateway-entry");
}

#[test]
fn entry_rejects_an_equal_program_from_a_different_allocation() {
    let exact_owner = owner("build:entry-mismatch");
    let image_program = non_clone_program(exact_owner.clone(), "verified");
    let entry_program = non_clone_program(exact_owner.clone(), "verified");
    assert_eq!(image_program, entry_program);
    assert!(!Arc::ptr_eq(&image_program, &entry_program));

    let image =
        Arc::new(DeploymentImage::try_new(image_program).expect("empty dependency set is valid"));
    let error = PinnedDeploymentEntry::try_new(
        image,
        FakeEntry {
            owner: exact_owner,
            program: entry_program,
            name: "forged-entry",
        },
    )
    .expect_err("value equality cannot substitute for the program Arc");

    assert_eq!(error, PinnedDeploymentEntryError::ProgramMismatch);
}

#[test]
fn entry_rejects_the_same_program_rebound_to_a_different_owner() {
    let image_owner = owner("build:image-owner");
    let entry_owner = owner("build:entry-owner");
    let program = non_clone_program(image_owner, "verified");
    let image = Arc::new(
        DeploymentImage::try_new(Arc::clone(&program)).expect("empty dependency set is valid"),
    );

    let error = PinnedDeploymentEntry::try_new(
        image,
        FakeEntry {
            owner: entry_owner,
            program,
            name: "rebound-entry",
        },
    )
    .expect_err("the same program allocation cannot be rebound to another owner");

    assert_eq!(error, PinnedDeploymentEntryError::OwnerMismatch);
}

fn non_clone_program(owner: DeploymentOwnerIdentity, label: &'static str) -> Arc<NonCloneProgram> {
    Arc::new(NonCloneProgram {
        owner,
        dependency_slots: Box::new([]),
        label,
    })
}
