use std::collections::{BTreeMap, BTreeSet};

use crate::{
    semantic::impl_method_declaration_name, ExpressionOwnerKey, ResolvedCallTargetFacts,
    SourceSymbolKey,
};

#[derive(Debug)]
pub(super) struct LocalCallGraph {
    edges: BTreeMap<SourceSymbolKey, BTreeSet<SourceSymbolKey>>,
}

impl LocalCallGraph {
    pub fn build(
        nodes: impl IntoIterator<Item = SourceSymbolKey>,
        targets: &ResolvedCallTargetFacts,
    ) -> Self {
        let mut edges = nodes
            .into_iter()
            .map(|node| (node, BTreeSet::new()))
            .collect::<BTreeMap<_, _>>();
        for (expression, target) in targets.iter() {
            let Some(caller) = expression_owner_key(expression.owner(), expression.module_path())
            else {
                continue;
            };
            let Some(callee) = target.source_callable_key() else {
                continue;
            };
            if edges.contains_key(&callee) {
                edges.entry(caller).or_default().insert(callee);
            }
        }
        Self { edges }
    }

    /// Tarjan emits sink components before their callers for caller -> callee
    /// edges, which is exactly the evaluation order required by effect import.
    pub fn callee_first_sccs(&self) -> Vec<Vec<SourceSymbolKey>> {
        let mut tarjan = Tarjan::new(&self.edges);
        for node in self.edges.keys() {
            if !tarjan.indices.contains_key(node) {
                tarjan.visit(node.clone());
            }
        }
        tarjan.components
    }
}

fn expression_owner_key(owner: &ExpressionOwnerKey, module_path: &str) -> Option<SourceSymbolKey> {
    match owner {
        ExpressionOwnerKey::Function(function) => Some(SourceSymbolKey::new(module_path, function)),
        ExpressionOwnerKey::ImplMethod { type_name, method } => Some(SourceSymbolKey::new(
            module_path,
            impl_method_declaration_name(type_name, method),
        )),
        ExpressionOwnerKey::Const(_)
        | ExpressionOwnerKey::Test(_)
        | ExpressionOwnerKey::DbIndexWhere { .. } => None,
    }
}

struct Tarjan<'a> {
    edges: &'a BTreeMap<SourceSymbolKey, BTreeSet<SourceSymbolKey>>,
    next_index: usize,
    indices: BTreeMap<SourceSymbolKey, usize>,
    lowlinks: BTreeMap<SourceSymbolKey, usize>,
    stack: Vec<SourceSymbolKey>,
    on_stack: BTreeSet<SourceSymbolKey>,
    components: Vec<Vec<SourceSymbolKey>>,
}

impl<'a> Tarjan<'a> {
    fn new(edges: &'a BTreeMap<SourceSymbolKey, BTreeSet<SourceSymbolKey>>) -> Self {
        Self {
            edges,
            next_index: 0,
            indices: BTreeMap::new(),
            lowlinks: BTreeMap::new(),
            stack: Vec::new(),
            on_stack: BTreeSet::new(),
            components: Vec::new(),
        }
    }

    fn visit(&mut self, node: SourceSymbolKey) {
        let index = self.next_index;
        self.next_index += 1;
        self.indices.insert(node.clone(), index);
        self.lowlinks.insert(node.clone(), index);
        self.stack.push(node.clone());
        self.on_stack.insert(node.clone());

        for callee in self.edges.get(&node).into_iter().flatten() {
            if !self.indices.contains_key(callee) {
                self.visit(callee.clone());
                let callee_lowlink = self.lowlinks[callee];
                self.lowlinks
                    .entry(node.clone())
                    .and_modify(|lowlink| *lowlink = (*lowlink).min(callee_lowlink));
            } else if self.on_stack.contains(callee) {
                let callee_index = self.indices[callee];
                self.lowlinks
                    .entry(node.clone())
                    .and_modify(|lowlink| *lowlink = (*lowlink).min(callee_index));
            }
        }

        if self.lowlinks[&node] != self.indices[&node] {
            return;
        }
        let mut component = Vec::new();
        loop {
            let member = self
                .stack
                .pop()
                .expect("Tarjan root must own a stack member");
            self.on_stack.remove(&member);
            component.push(member.clone());
            if member == node {
                break;
            }
        }
        component.sort();
        self.components.push(component);
    }
}
