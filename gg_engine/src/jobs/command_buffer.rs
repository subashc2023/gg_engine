use crate::scene::Scene;

type SpawnFn = Box<dyn FnOnce(&mut Scene) + Send>;
type InsertFn = Box<dyn FnOnce(&mut hecs::World) + Send>;

/// Deferred structural ECS changes — integrates with existing `pending_destroy` pattern.
///
/// Accumulate spawn/destroy/insert operations during parallel work, then flush
/// them on the main thread when `&mut Scene` is available.
pub struct CommandBuffer {
    destroys: Vec<u64>,
    spawns: Vec<SpawnFn>,
    inserts: Vec<InsertFn>,
}

impl CommandBuffer {
    pub fn new() -> Self {
        Self {
            destroys: Vec::new(),
            spawns: Vec::new(),
            inserts: Vec::new(),
        }
    }

    /// Queue an entity for destruction by UUID.
    pub fn destroy_entity(&mut self, uuid: u64) {
        self.destroys.push(uuid);
    }

    /// Queue a spawn operation (runs with `&mut Scene`).
    pub fn spawn(&mut self, f: impl FnOnce(&mut Scene) + Send + 'static) {
        self.spawns.push(Box::new(f));
    }

    /// Queue a component insertion on an existing hecs entity.
    pub fn insert_component<T: hecs::Component>(&mut self, entity: hecs::Entity, component: T) {
        self.inserts.push(Box::new(move |world| {
            let _ = world.insert_one(entity, component);
        }));
    }

    /// Returns `true` if no operations are queued.
    pub fn is_empty(&self) -> bool {
        self.destroys.is_empty() && self.spawns.is_empty() && self.inserts.is_empty()
    }

    /// Flush all queued operations into the scene.
    /// Destroys go through `Scene::queue_entity_destroy` + `flush_pending_destroys`.
    pub fn flush(self, scene: &mut Scene) {
        let has_destroys = !self.destroys.is_empty();
        for uuid in self.destroys {
            scene.queue_entity_destroy(uuid);
        }
        if has_destroys {
            scene.flush_pending_destroys();
        }
        for f in self.spawns {
            f(scene);
        }
        let world = scene.world_mut();
        for f in self.inserts {
            f(world);
        }
    }
}

impl Default for CommandBuffer {
    fn default() -> Self {
        Self::new()
    }
}
