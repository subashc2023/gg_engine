use std::fmt;

/// Lightweight handle to an entity in a [`Scene`](super::Scene).
///
/// This is a `Copy` newtype over [`hecs::Entity`]. It does not store a
/// reference to the scene — all operations that read or mutate
/// components go through [`Scene`](super::Scene) methods.
///
/// Obtain an `Entity` via [`Scene::create_entity`](super::Scene::create_entity)
/// or [`Scene::create_entity_with_tag`](super::Scene::create_entity_with_tag).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    handle: hecs::Entity,
}

impl Entity {
    /// Wrap a raw `hecs::Entity`. Primarily for internal use.
    pub(crate) fn new(handle: hecs::Entity) -> Self {
        Self { handle }
    }

    /// The underlying [`hecs::Entity`] handle.
    pub fn handle(&self) -> hecs::Entity {
        self.handle
    }

    /// Unique integer ID of this entity (from hecs).
    pub fn id(&self) -> u32 {
        self.handle.id()
    }
}

impl fmt::Debug for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entity({})", self.handle.id())
    }
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entity({})", self.handle.id())
    }
}
