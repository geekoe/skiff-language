#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationCompilePolicy<'a> {
    Package { package_id: &'a str },
    Service { service_id: &'a str },
}
