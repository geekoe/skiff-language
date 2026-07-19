#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackageCompilePolicy<'a> {
    package_id: &'a str,
}

impl<'a> PackageCompilePolicy<'a> {
    pub fn new(package_id: &'a str) -> Self {
        Self { package_id }
    }

    pub fn package_id(self) -> &'a str {
        self.package_id
    }
}
