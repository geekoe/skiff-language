use std::collections::BTreeMap;

use crate::{PackageDependency, ResolvedServiceDependencies, ServiceIngressSeed};

pub trait PublicationInputMetadata {
    fn package_dependencies(&self) -> &[PackageDependency];
}

#[derive(Clone, Copy)]
pub struct PublicationInputCore<'a, P: ?Sized> {
    pub publication: &'a P,
    pub package_aliases: &'a BTreeMap<String, Vec<String>>,
    pub package_dependencies: &'a [PackageDependency],
}

pub struct PackagePublicationInput<'a, P: ?Sized> {
    pub core: PublicationInputCore<'a, P>,
    pub package_id: &'a str,
}

pub struct ServicePublicationInput<'a, P: ?Sized> {
    pub core: PublicationInputCore<'a, P>,
    pub service_id: &'a str,
    pub service_dependencies: ResolvedServiceDependencies,
    pub service_ingress: ServiceIngressSeed,
}

pub enum PublicationInput<'a, P: ?Sized> {
    Package(PackagePublicationInput<'a, P>),
    Service(ServicePublicationInput<'a, P>),
}

impl<'a, P: PublicationInputMetadata + ?Sized> PublicationInputCore<'a, P> {
    pub fn new(publication: &'a P, package_aliases: &'a BTreeMap<String, Vec<String>>) -> Self {
        Self {
            publication,
            package_aliases,
            package_dependencies: publication.package_dependencies(),
        }
    }
}

impl<'a, P: PublicationInputMetadata + ?Sized> PackagePublicationInput<'a, P> {
    pub fn new(
        publication: &'a P,
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        package_id: &'a str,
    ) -> Self {
        Self {
            core: PublicationInputCore::new(publication, package_aliases),
            package_id,
        }
    }
}

impl<'a, P: PublicationInputMetadata + ?Sized> ServicePublicationInput<'a, P> {
    pub fn new_with_service_id(
        publication: &'a P,
        package_aliases: &'a BTreeMap<String, Vec<String>>,
        service_id: &'a str,
        service_dependencies: ResolvedServiceDependencies,
        service_ingress: ServiceIngressSeed,
    ) -> Self {
        Self {
            core: PublicationInputCore::new(publication, package_aliases),
            service_id,
            service_dependencies,
            service_ingress,
        }
    }
}
