//! Writable-place model (design §3.1, R-195).
//!
//! A writable place is a name/member/index path whose root is one of exactly
//! three writable roots:
//!
//! - a local `var` binding (`WritableRoot::VarBinding`),
//! - a currently valid `inout` loan parameter (`WritableRoot::InOutParam`),
//! - an Actor method `self.field` (`WritableRoot::ActorSelfField`).
//!
//! Every other root — `let` bindings, ordinary parameters, top-level `consts`,
//! loop/pattern/with bindings — makes any derived member/index path
//! unwritable. The path must be an exact selector chain (no aliasing
//! constructs such as calls, ternary or value blocks).

use crate::shared::ast::Expr;

/// One step of an exact place path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Selector {
    Field(String),
    Index,
}

/// The writable root of a place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WritableRoot {
    /// Local `var` binding by name.
    VarBinding(String),
    /// A currently valid `inout` loan parameter (callee side), by parameter
    /// index. No inout usage exists in the current repositories yet, so this
    /// root is only ever matched, never constructed.
    #[allow(dead_code)]
    InOutParam(usize),
    /// Actor method `self.field`, by field name.
    ActorSelfField(String),
}

/// One exact writable place: root plus the exact selector path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WritablePlace {
    pub root: WritableRoot,
    pub path: Vec<Selector>,
}

impl WritablePlace {
    /// The root binding name for scope lookup.
    pub fn root_name(&self) -> &str {
        match &self.root {
            WritableRoot::VarBinding(name) => name,
            WritableRoot::InOutParam(_) => "",
            WritableRoot::ActorSelfField(_) => "self",
        }
    }

    /// Two places overlap when they share a root and one path is a prefix of
    /// the other (or they are identical).
    pub fn overlaps(&self, other: &WritablePlace) -> bool {
        same_root(&self.root, &other.root)
            && (path_prefix(&self.path, &other.path) || path_prefix(&other.path, &self.path))
    }
}

fn same_root(left: &WritableRoot, right: &WritableRoot) -> bool {
    match (left, right) {
        (WritableRoot::VarBinding(a), WritableRoot::VarBinding(b)) => a == b,
        (WritableRoot::InOutParam(a), WritableRoot::InOutParam(b)) => a == b,
        (WritableRoot::ActorSelfField(a), WritableRoot::ActorSelfField(b)) => a == b,
        _ => false,
    }
}

fn path_prefix(left: &[Selector], right: &[Selector]) -> bool {
    left.len() <= right.len() && left.iter().zip(right.iter()).all(|(a, b)| a == b)
}

/// Computes the exact name/member/index place for an expression.
///
/// Returns `None` for aliasing or non-place constructs (calls, ternary, value
/// blocks, literals, `self` itself, ...). `self.field` paths produce
/// `WritableRoot::ActorSelfField`; bare `self` is not a place.
pub fn place_from_expr(expr: &Expr) -> Option<WritablePlace> {
    match expr {
        Expr::Identifier(name) if name != "self" => Some(WritablePlace {
            root: WritableRoot::VarBinding(name.clone()),
            path: Vec::new(),
        }),
        Expr::Field { object, field } => {
            if matches!(object.as_ref(), Expr::Identifier(name) if name == "self") {
                Some(WritablePlace {
                    root: WritableRoot::ActorSelfField(field.clone()),
                    path: Vec::new(),
                })
            } else {
                let object = place_from_expr(object)?;
                let mut path = object.path;
                path.push(Selector::Field(field.clone()));
                Some(WritablePlace { root: object.root, path })
            }
        }
        Expr::Generic { callee, .. } => place_from_expr(callee),
        _ => None,
    }
}

/// Selector path only, for expressions whose root is not a binding (used to
/// compare loaned paths against arbitrary reads).
pub fn selectors_from_expr(expr: &Expr) -> Option<Vec<Selector>> {
    match expr {
        Expr::Identifier(_) => Some(Vec::new()),
        Expr::Field { object, field } => {
            let mut selectors = selectors_from_expr(object)?;
            selectors.push(Selector::Field(field.clone()));
            Some(selectors)
        }
        Expr::Generic { callee, .. } => selectors_from_expr(callee),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ident(name: &str) -> Expr {
        Expr::Identifier(name.to_string())
    }

    fn field(object: Expr, name: &str) -> Expr {
        Expr::Field {
            object: Box::new(object),
            field: name.to_string(),
        }
    }

    #[test]
    fn exact_paths_and_overlap() {
        let place = place_from_expr(&field(field(ident("x"), "items"), "first")).unwrap();
        assert_eq!(place.root, WritableRoot::VarBinding("x".to_string()));
        assert_eq!(place.path.len(), 2);

        let root_place = place_from_expr(&ident("x")).unwrap();
        assert!(root_place.overlaps(&place));
        assert!(place.overlaps(&root_place));

        let other = place_from_expr(&field(ident("x"), "other")).unwrap();
        assert!(!place.overlaps(&other));

        let other_var = place_from_expr(&field(ident("y"), "items")).unwrap();
        assert!(!place.overlaps(&other_var));
    }

    #[test]
    fn self_field_is_an_actor_root() {
        let place = place_from_expr(&field(field(ident("self"), "buffer"), "head")).unwrap();
        assert_eq!(place.root, WritableRoot::ActorSelfField("buffer".to_string()));
        assert_eq!(place.path, vec![Selector::Field("head".to_string())]);
    }

    #[test]
    fn non_places_are_rejected() {
        assert!(place_from_expr(&ident("self")).is_none());
        assert!(place_from_expr(&Expr::Literal(crate::shared::ast::Literal::Number(1.0))).is_none());
    }
}
