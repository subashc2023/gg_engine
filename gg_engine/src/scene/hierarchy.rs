use super::{
    Entity, IdComponent, RelationshipComponent, RigidBody2DComponent, Scene, TagComponent,
    TransformComponent,
};
use std::collections::HashMap;

impl Scene {
    // -----------------------------------------------------------------
    // Hierarchy (parent-child relationships)
    // -----------------------------------------------------------------

    /// Set `child` as a child of `parent`. Detaches from current parent if any.
    ///
    /// If `preserve_world_transform` is `true`, the child's local transform is
    /// adjusted so its world position stays the same.
    ///
    /// Returns `false` if the operation would create a cycle.
    pub fn set_parent(
        &mut self,
        child: Entity,
        parent: Entity,
        preserve_world_transform: bool,
    ) -> bool {
        let child_uuid = match self.get_component::<IdComponent>(child) {
            Some(id) => id.id.raw(),
            None => return false,
        };
        let parent_uuid = match self.get_component::<IdComponent>(parent) {
            Some(id) => id.id.raw(),
            None => return false,
        };

        // Prevent self-parenting.
        if child_uuid == parent_uuid {
            return false;
        }

        // Prevent cycles.
        if self.is_ancestor_of(child_uuid, parent_uuid) {
            return false;
        }

        // Compute world transform before reparenting.
        let world_mat = if preserve_world_transform {
            Some(self.get_world_transform(child))
        } else {
            None
        };

        // Detach from current parent.
        self.detach_from_parent_impl(child, child_uuid, false);

        // Add to new parent.
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(parent) {
            rel.children.push(child_uuid);
        }
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(child) {
            rel.parent = Some(parent_uuid);
        }

        // Adjust local transform to preserve world position.
        if let Some(world_mat) = world_mat {
            let parent_world = self.get_world_transform(parent);
            let local = parent_world.inverse() * world_mat;
            self.decompose_and_set_local_transform(child, local);
        }

        // Warn if physics entity is parented.
        if self.has_component::<RigidBody2DComponent>(child) {
            log::warn!("Entity with RigidBody2D parented — physics may not behave correctly");
        }

        true
    }

    /// Remove an entity from its parent, making it a root entity.
    ///
    /// If `preserve_world_transform` is `true`, the local transform is adjusted
    /// so the entity's world position stays the same.
    pub fn detach_from_parent(&mut self, entity: Entity, preserve_world_transform: bool) {
        let uuid = match self.get_component::<IdComponent>(entity) {
            Some(id) => id.id.raw(),
            None => return,
        };
        self.detach_from_parent_impl(entity, uuid, preserve_world_transform);
    }

    fn detach_from_parent_impl(
        &mut self,
        entity: Entity,
        entity_uuid: u64,
        preserve_world_transform: bool,
    ) {
        let parent_uuid = self
            .get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent);

        let Some(parent_uuid) = parent_uuid else {
            return;
        };

        // Compute world transform before detaching.
        let world_mat = if preserve_world_transform {
            Some(self.get_world_transform(entity))
        } else {
            None
        };

        // Remove from parent's children list.
        if let Some(parent_entity) = self.find_entity_by_uuid(parent_uuid) {
            if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(parent_entity) {
                rel.children.retain(|&c| c != entity_uuid);
            }
        }

        // Clear parent reference.
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(entity) {
            rel.parent = None;
        }

        // Restore world transform.
        if let Some(world_mat) = world_mat {
            self.decompose_and_set_local_transform(entity, world_mat);
        }
    }

    /// Compute the world-space transform for an entity by walking the parent chain.
    ///
    /// No caching — walks up from entity to root each call. Fine for scenes with
    /// O(100s) of entities and hierarchy depth ~3.
    pub fn get_world_transform(&self, entity: Entity) -> glam::Mat4 {
        let local = self
            .get_component::<TransformComponent>(entity)
            .map(|tc| tc.get_transform())
            .unwrap_or(glam::Mat4::IDENTITY);

        let parent_uuid = self
            .get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent);

        match parent_uuid {
            Some(puuid) => {
                if let Some(parent_entity) = self.find_entity_by_uuid(puuid) {
                    self.get_world_transform(parent_entity) * local
                } else {
                    local
                }
            }
            None => local,
        }
    }

    /// Compute the world transform for `entity`, using and populating `cache`.
    ///
    /// Same logic as [`get_world_transform`](Self::get_world_transform) but
    /// avoids redundant parent-chain walks when many entities share ancestors.
    fn get_world_transform_cached(
        &self,
        entity: Entity,
        cache: &mut HashMap<hecs::Entity, glam::Mat4>,
    ) -> glam::Mat4 {
        if let Some(&cached) = cache.get(&entity.handle()) {
            return cached;
        }

        let local = self
            .get_component::<TransformComponent>(entity)
            .map(|tc| tc.get_transform())
            .unwrap_or(glam::Mat4::IDENTITY);

        let parent_uuid = self
            .get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent);

        let world = match parent_uuid {
            Some(puuid) => {
                if let Some(parent_entity) = self.find_entity_by_uuid(puuid) {
                    self.get_world_transform_cached(parent_entity, cache) * local
                } else {
                    local
                }
            }
            None => local,
        };

        cache.insert(entity.handle(), world);
        world
    }

    /// Build a cache of world transforms for all entities.
    ///
    /// Uses persistent caching with snapshot-based dirty detection: on each
    /// call, every entity's local transform and parent UUID are compared with
    /// a snapshot taken when the cache was last built. If nothing changed (and
    /// the entity count is the same), the cached transforms are returned
    /// without recomputation.
    ///
    /// For scenes above [`PAR_THRESHOLD`](crate::jobs::parallel::PAR_THRESHOLD),
    /// root subtrees are processed in parallel via rayon.
    pub(super) fn build_world_transform_cache(&self) -> HashMap<hecs::Entity, glam::Mat4> {
        // --- Dirty detection: compare current transforms against cached snapshots ---
        let needs_rebuild = {
            let snapshots = self.transform_snapshots.borrow();
            if snapshots.len() != self.world.len() as usize {
                true
            } else {
                let mut changed = false;
                for (handle, tc, rel) in self
                    .world
                    .query::<(hecs::Entity, &TransformComponent, &RelationshipComponent)>()
                    .iter()
                {
                    if let Some(&(cached_local, cached_parent)) = snapshots.get(&handle) {
                        if tc.get_transform() != cached_local || rel.parent != cached_parent {
                            changed = true;
                            break;
                        }
                    } else {
                        changed = true;
                        break;
                    }
                }
                changed
            }
        };

        if !needs_rebuild {
            return self.transform_cache.borrow().clone();
        }

        // --- Full rebuild ---
        let cache = self.build_world_transform_cache_impl();

        // Take a snapshot of current local transforms for next frame's dirty detection.
        let mut snapshots =
            HashMap::with_capacity(self.world.len() as usize);
        for (handle, tc, rel) in self
            .world
            .query::<(hecs::Entity, &TransformComponent, &RelationshipComponent)>()
            .iter()
        {
            snapshots.insert(handle, (tc.get_transform(), rel.parent));
        }
        *self.transform_snapshots.borrow_mut() = snapshots;
        *self.transform_cache.borrow_mut() = cache.clone();
        cache
    }

    /// Full world-transform rebuild (parallel or sequential depending on entity count).
    fn build_world_transform_cache_impl(&self) -> HashMap<hecs::Entity, glam::Mat4> {
        let entity_count = self.world.len() as usize;
        if entity_count < crate::jobs::parallel::PAR_THRESHOLD {
            return self.build_world_transform_cache_sequential();
        }

        // -- Extract phase (sequential): copy component data into owned Vecs --
        // Children UUIDs stored in a flat buffer (one allocation) rather than
        // a Vec per entity (N allocations). Each entity stores a start/len range.
        struct EntityData {
            handle: hecs::Entity,
            local_transform: glam::Mat4,
            parent_uuid: Option<u64>,
            children_start: u32,
            children_len: u32,
        }

        let mut data: Vec<EntityData> = Vec::with_capacity(entity_count);
        let mut children_buf: Vec<u64> = Vec::new();
        let mut uuid_to_idx: HashMap<u64, usize> = HashMap::with_capacity(entity_count);

        for (handle, id, tc, rel) in self
            .world
            .query::<(
                hecs::Entity,
                &IdComponent,
                &TransformComponent,
                &RelationshipComponent,
            )>()
            .iter()
        {
            let idx = data.len();
            uuid_to_idx.insert(id.id.raw(), idx);
            let children_start = children_buf.len() as u32;
            children_buf.extend_from_slice(&rel.children);
            data.push(EntityData {
                handle,
                local_transform: tc.get_transform(),
                parent_uuid: rel.parent,
                children_start,
                children_len: rel.children.len() as u32,
            });
        }

        // Identify root entities (no parent).
        let roots: Vec<usize> = data
            .iter()
            .enumerate()
            .filter(|(_, e)| e.parent_uuid.is_none())
            .map(|(i, _)| i)
            .collect();

        // -- Process phase (parallel): each root subtree computed independently --
        fn compute_subtree(
            idx: usize,
            parent_world: glam::Mat4,
            data: &[EntityData],
            children_buf: &[u64],
            uuid_to_idx: &HashMap<u64, usize>,
            results: &mut Vec<(hecs::Entity, glam::Mat4)>,
        ) {
            let entity = &data[idx];
            let world = parent_world * entity.local_transform;
            results.push((entity.handle, world));
            let start = entity.children_start as usize;
            let end = start + entity.children_len as usize;
            for &child_uuid in &children_buf[start..end] {
                if let Some(&child_idx) = uuid_to_idx.get(&child_uuid) {
                    compute_subtree(child_idx, world, data, children_buf, uuid_to_idx, results);
                }
            }
        }

        use rayon::prelude::*;
        let data_ref = &data;
        let children_buf_ref = &children_buf;
        let uuid_to_idx_ref = &uuid_to_idx;
        let sub_results: Vec<Vec<(hecs::Entity, glam::Mat4)>> = crate::jobs::pool().install(|| {
            roots
                .par_iter()
                .map(|&root_idx| {
                    let mut results = Vec::new();
                    compute_subtree(
                        root_idx,
                        glam::Mat4::IDENTITY,
                        data_ref,
                        children_buf_ref,
                        uuid_to_idx_ref,
                        &mut results,
                    );
                    results
                })
                .collect()
        });

        // -- Merge phase (sequential) --
        let mut cache = HashMap::with_capacity(entity_count);
        for sub in sub_results {
            for (handle, transform) in sub {
                cache.insert(handle, transform);
            }
        }
        cache
    }

    /// Sequential fallback for small scenes (below parallel threshold).
    fn build_world_transform_cache_sequential(&self) -> HashMap<hecs::Entity, glam::Mat4> {
        let mut cache = HashMap::with_capacity(self.world.len() as usize);
        let entities: Vec<hecs::Entity> = self.world.query::<hecs::Entity>().iter().collect();
        for handle in entities {
            self.get_world_transform_cached(Entity::new(handle), &mut cache);
        }
        cache
    }

    /// Get the children UUIDs of an entity.
    pub fn get_children(&self, entity: Entity) -> Vec<u64> {
        self.get_component::<RelationshipComponent>(entity)
            .map(|r| r.children.clone())
            .unwrap_or_default()
    }

    /// Get the parent UUID of an entity.
    pub fn get_parent(&self, entity: Entity) -> Option<u64> {
        self.get_component::<RelationshipComponent>(entity)
            .and_then(|r| r.parent)
    }

    /// Move a child entity to a specific index within its parent's children list.
    ///
    /// No-op if the entity has no parent or the UUID is not found in the children.
    pub fn reorder_child(&mut self, child_uuid: u64, new_index: usize) {
        let Some(child_entity) = self.find_entity_by_uuid(child_uuid) else {
            return;
        };
        let parent_uuid = match self.get_parent(child_entity) {
            Some(p) => p,
            None => return,
        };
        let Some(parent_entity) = self.find_entity_by_uuid(parent_uuid) else {
            return;
        };
        if let Some(mut rel) = self.get_component_mut::<RelationshipComponent>(parent_entity) {
            let Some(current_pos) = rel.children.iter().position(|&c| c == child_uuid) else {
                return;
            };
            rel.children.remove(current_pos);
            let clamped = new_index.min(rel.children.len());
            rel.children.insert(clamped, child_uuid);
        }
    }

    /// Return all root entities (entities without a parent), sorted by entity ID.
    pub fn root_entities(&self) -> Vec<(Entity, String)> {
        let mut entities: Vec<(Entity, String)> = self
            .world
            .query::<(hecs::Entity, &TagComponent, &RelationshipComponent)>()
            .iter()
            .filter(|(_, _, rel)| rel.parent.is_none())
            .map(|(handle, tag, _)| (Entity::new(handle), tag.tag.clone()))
            .collect();
        entities.sort_by_key(|(e, _): &(Entity, String)| e.id());
        entities
    }

    /// Check if `ancestor_uuid` is an ancestor of `entity_uuid`.
    ///
    /// Used for cycle detection in [`set_parent`](Self::set_parent).
    pub fn is_ancestor_of(&self, ancestor_uuid: u64, entity_uuid: u64) -> bool {
        let mut current = entity_uuid;
        let mut visited = std::collections::HashSet::new();
        loop {
            if !visited.insert(current) {
                // Cycle detected — treat as not an ancestor.
                return false;
            }
            if let Some(entity) = self.find_entity_by_uuid(current) {
                if let Some(parent) = self.get_parent(entity) {
                    if parent == ancestor_uuid {
                        return true;
                    }
                    current = parent;
                } else {
                    return false;
                }
            } else {
                return false;
            }
        }
    }

    /// Decompose a 4x4 matrix into translation/rotation/scale and set on the entity.
    fn decompose_and_set_local_transform(&mut self, entity: Entity, mat: glam::Mat4) {
        let (scale, rotation, translation) = mat.to_scale_rotation_translation();
        if let Some(mut tc) = self.get_component_mut::<TransformComponent>(entity) {
            tc.translation = translation;
            tc.rotation = rotation;
            tc.scale = scale;
        }
    }
}
