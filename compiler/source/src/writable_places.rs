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
    Index(IndexSelector),
}

/// Stable selector identity used only for static overlap proofs. Any selector
/// that is not a direct literal remains `Dynamic` and overlaps every other
/// index at the same path position.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum IndexSelector {
    StringLiteral(String),
    NumberLiteral(u64),
    Dynamic,
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
    left.len() <= right.len()
        && left
            .iter()
            .zip(right.iter())
            .all(|(left, right)| selectors_may_overlap(left, right))
}

fn selectors_may_overlap(left: &Selector, right: &Selector) -> bool {
    match (left, right) {
        (Selector::Field(left), Selector::Field(right)) => left == right,
        (Selector::Index(IndexSelector::Dynamic), Selector::Index(_))
        | (Selector::Index(_), Selector::Index(IndexSelector::Dynamic)) => true,
        (Selector::Index(left), Selector::Index(right)) => left == right,
        (Selector::Field(_), Selector::Index(_)) | (Selector::Index(_), Selector::Field(_)) => {
            false
        }
    }
}

fn index_selector(expr: &Expr) -> IndexSelector {
    match expr {
        Expr::Literal(crate::shared::ast::Literal::String(value)) => {
            IndexSelector::StringLiteral(value.clone())
        }
        Expr::Literal(crate::shared::ast::Literal::Number(value)) => {
            IndexSelector::NumberLiteral(value.to_bits())
        }
        Expr::Literal(crate::shared::ast::Literal::Bool(_))
        | Expr::Literal(crate::shared::ast::Literal::Null)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Ternary { .. }
        | Expr::Call { .. }
        | Expr::Generic { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Field { .. }
        | Expr::Index { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::MapLiteral { .. }
        | Expr::ArrayLiteral { .. }
        | Expr::Patch { .. }
        | Expr::ValueBlock(_)
        | Expr::ConcurrentValue(_)
        | Expr::Timeout { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Dispatch { .. } => IndexSelector::Dynamic,
    }
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
                Some(WritablePlace {
                    root: object.root,
                    path,
                })
            }
        }
        Expr::Index { object, index } => {
            let object = place_from_expr(object)?;
            let mut path = object.path;
            path.push(Selector::Index(index_selector(index)));
            Some(WritablePlace {
                root: object.root,
                path,
            })
        }
        Expr::Generic { callee, .. } => place_from_expr(callee),
        Expr::Literal(_)
        | Expr::Identifier(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Ternary { .. }
        | Expr::Call { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::MapLiteral { .. }
        | Expr::ArrayLiteral { .. }
        | Expr::Patch { .. }
        | Expr::ValueBlock(_)
        | Expr::ConcurrentValue(_)
        | Expr::Timeout { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Dispatch { .. } => None,
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
        Expr::Index { object, index } => {
            let mut selectors = selectors_from_expr(object)?;
            selectors.push(Selector::Index(index_selector(index)));
            Some(selectors)
        }
        Expr::Generic { callee, .. } => selectors_from_expr(callee),
        Expr::Literal(_)
        | Expr::DependencySourceAddress(_)
        | Expr::Binary { .. }
        | Expr::Unary { .. }
        | Expr::Ternary { .. }
        | Expr::Call { .. }
        | Expr::InterfaceBox { .. }
        | Expr::Record { .. }
        | Expr::ObjectLiteral { .. }
        | Expr::MapLiteral { .. }
        | Expr::ArrayLiteral { .. }
        | Expr::Patch { .. }
        | Expr::ValueBlock(_)
        | Expr::ConcurrentValue(_)
        | Expr::Timeout { .. }
        | Expr::Throw { .. }
        | Expr::Rethrow { .. }
        | Expr::Catch { .. }
        | Expr::DbOperation(_)
        | Expr::DbQuery(_)
        | Expr::DbTransaction(_)
        | Expr::DbLeaseClaim(_)
        | Expr::DbLeaseRead(_)
        | Expr::Dispatch { .. } => None,
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

    fn index(object: Expr, selector: Expr) -> Expr {
        Expr::Index {
            object: Box::new(object),
            index: Box::new(selector),
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
        assert_eq!(
            place.root,
            WritableRoot::ActorSelfField("buffer".to_string())
        );
        assert_eq!(place.path, vec![Selector::Field("head".to_string())]);
    }

    #[test]
    fn non_places_are_rejected() {
        assert!(place_from_expr(&ident("self")).is_none());
        assert!(
            place_from_expr(&Expr::Literal(crate::shared::ast::Literal::Number(1.0))).is_none()
        );
    }

    #[test]
    fn literal_indexes_are_exact_and_dynamic_indexes_overlap_conservatively() {
        let zero = place_from_expr(&index(
            ident("items"),
            Expr::Literal(crate::shared::ast::Literal::Number(0.0)),
        ))
        .unwrap();
        let one = place_from_expr(&index(
            ident("items"),
            Expr::Literal(crate::shared::ast::Literal::Number(1.0)),
        ))
        .unwrap();
        let dynamic = place_from_expr(&index(ident("items"), ident("position"))).unwrap();

        assert!(!zero.overlaps(&one));
        assert!(zero.overlaps(&dynamic));
        assert!(dynamic.overlaps(&one));
    }
}
