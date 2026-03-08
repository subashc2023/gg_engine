use gg_engine::prelude::Entity;

/// Multi-entity selection state for the editor.
#[derive(Default)]
pub(crate) struct Selection {
    entities: Vec<Entity>,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entities.len()
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(&entity)
    }

    /// Returns the single selected entity, or `None` if 0 or 2+ are selected.
    pub fn single(&self) -> Option<Entity> {
        if self.entities.len() == 1 {
            Some(self.entities[0])
        } else {
            None
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = Entity> + '_ {
        self.entities.iter().copied()
    }

    /// Replace selection with a single entity.
    pub fn set(&mut self, entity: Entity) {
        self.entities.clear();
        self.entities.push(entity);
    }

    /// Toggle entity in selection (Ctrl+click).
    pub fn toggle(&mut self, entity: Entity) {
        if let Some(pos) = self.entities.iter().position(|&e| e == entity) {
            self.entities.remove(pos);
        } else {
            self.entities.push(entity);
        }
    }

    /// Add entity if not already present.
    pub fn add(&mut self, entity: Entity) {
        if !self.entities.contains(&entity) {
            self.entities.push(entity);
        }
    }

    /// Remove a specific entity from selection.
    pub fn remove(&mut self, entity: Entity) {
        self.entities.retain(|&e| e != entity);
    }

    /// Clear selection.
    pub fn clear(&mut self) {
        self.entities.clear();
    }

    /// Remove entities that don't satisfy the predicate.
    pub fn retain(&mut self, f: impl FnMut(&Entity) -> bool) {
        self.entities.retain(f);
    }
}
