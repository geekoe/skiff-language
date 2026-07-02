use crate::{LinkedImageActivationFacts, LinkedProgramImage, RuntimeProgramIdentity};

#[derive(Debug, Clone)]
pub struct LinkedProgramImageBuild {
    pub identity: RuntimeProgramIdentity,
    pub image: LinkedProgramImage,
    pub activation_facts: LinkedImageActivationFacts,
}

impl LinkedProgramImageBuild {
    pub(super) fn new(
        identity: RuntimeProgramIdentity,
        image: LinkedProgramImage,
        activation_facts: LinkedImageActivationFacts,
    ) -> Self {
        Self {
            identity,
            image,
            activation_facts,
        }
    }
}
