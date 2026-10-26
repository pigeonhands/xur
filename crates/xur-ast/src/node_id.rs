use std::fmt;

/// Identifies an AST node that later passes record facts about, such as
/// what a path resolves to (rustc's `NodeId`).
///
/// Only nodes that define or refer to names carry one: items, enum variants,
/// generic parameters, patterns and paths.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(u32);

impl NodeId {
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Debug for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Hands out [`NodeId`]s. One generator is shared by every file parsed in a
/// session so ids are unique across modules.
#[derive(Debug, Default)]
pub struct NodeIdGen {
    next: u32,
}

impl NodeIdGen {
    pub fn next_id(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next = self.next.checked_add(1).expect("too many AST nodes");
        id
    }
}
