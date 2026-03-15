# Jobs System (Multi-Threaded ECS)

The jobs system provides rayon-based parallelism for ECS workloads. It lives in `gg_engine/src/jobs/` and consists of three modules: thread pool management, parallel query helpers, and a deferred command buffer.

## Architecture: Extract-Process-Writeback

`Scene` is `!Send` and `!Sync` because it contains `mlua::Lua`, `rapier2d::PhysicsWorld2D`, and `kira::AudioEngine` — all single-threaded libraries. This means `&Scene` cannot be shared across threads.

The solution is **Extract-Process-Writeback** (EPW):

1. **Extract** (sequential): Query `hecs::World`, copy relevant component data into owned `Vec<WorkItem>`. This releases all borrows on the World.
2. **Process** (parallel): `rayon::par_iter_mut()` on the owned Vec. No World borrow is held — pure computation on owned data. Thread-safe by construction.
3. **Writeback** (sequential): Iterate the Vec, write computed results back into the World via `get::<&mut T>()`.

Each phase holds its borrow exclusively. The parallel phase operates on owned data only. No `unsafe` code is needed.

```
 Sequential         Parallel              Sequential
┌──────────┐   ┌─────────────────┐   ┌──────────────┐
│  Extract  │──▶│  par_iter_mut   │──▶│  Writeback   │
│ (query +  │   │  (owned data,   │   │ (write back  │
│  copy)    │   │   no borrows)   │   │  to World)   │
└──────────┘   └─────────────────┘   └──────────────┘
```

## Module Structure

```
gg_engine/src/jobs/
    mod.rs              — Thread pool init, pool(), worker_count()
    parallel.rs         — par_for_each_mut, par_extract_map, PAR_THRESHOLD
    command_buffer.rs   — Deferred spawn/despawn/insert
```

## Thread Pool

**File:** `jobs/mod.rs`

The global thread pool is initialized once via `jobs::init()`, called automatically during `EngineRunner::resumed()` after window creation, before Vulkan init.

```rust
use gg_engine::jobs;

jobs::init();                    // Called automatically by the engine
jobs::pool();                    // &'static rayon::ThreadPool
jobs::worker_count();            // Number of worker threads (N-1 cores)
```

- Uses `OnceLock<rayon::ThreadPool>` for safe one-time initialization
- Creates N-1 worker threads where N = `available_parallelism()` (main thread participates via `pool().install()`)
- Worker threads are named `gg-worker-0`, `gg-worker-1`, etc. (used by profiling to route results)
- Minimum 1 worker thread, fallback to 3 if parallelism detection fails

## Parallel Helpers

**File:** `jobs/parallel.rs`

### PAR_THRESHOLD

```rust
pub const PAR_THRESHOLD: usize = 64;
```

Minimum item count before parallelizing. Below this threshold, all helpers fall back to sequential iteration to avoid rayon dispatch overhead.

**Performance note:** In release builds with lightweight per-entity work (e.g. simple float arithmetic), the crossover point where parallelism pays off is significantly higher than 64 — typically 5000+ entities on modern hardware. The threshold is set conservatively low for correctness; future tuning may raise it or make it configurable. The overhead comes from: (1) extracting component data into owned Vecs, (2) rayon thread scheduling, (3) writing results back. For the EPW pattern to win, the parallel phase must dominate these sequential costs.

### par_for_each_mut

```rust
pub fn par_for_each_mut<T, F>(items: &mut [T], f: F)
where
    T: Send,
    F: Fn(&mut T) + Send + Sync,
```

Process items in-place in parallel. Falls back to sequential below `PAR_THRESHOLD`. All work runs on the engine's thread pool via `pool().install()`.

### par_extract_map

```rust
pub fn par_extract_map<T, R, P>(items: Vec<T>, process: P) -> Vec<R>
where
    T: Send,
    R: Send,
    P: Fn(T) -> R + Send + Sync,
```

Transform items in parallel, consuming the input Vec and producing a new Vec of results.

## Command Buffer

**File:** `jobs/command_buffer.rs`

Accumulates structural ECS changes during parallel work phases for deferred execution on the main thread.

```rust
use gg_engine::jobs::command_buffer::CommandBuffer;

let mut cmds = CommandBuffer::new();
cmds.destroy_entity(uuid);                              // Queue destruction by UUID
cmds.spawn(|scene| { scene.create_entity(); });         // Queue a spawn operation
cmds.insert_component(hecs_entity, MyComponent { .. }); // Queue component insertion
cmds.is_empty();                                        // Check if any ops queued
cmds.flush(scene);                                      // Apply all ops to the scene
```

- Destroys go through `Scene::queue_entity_destroy()` + `flush_pending_destroys()` (existing deferred destruction pattern)
- Spawns receive `&mut Scene` during flush
- Inserts receive `&mut hecs::World` during flush

## Parallelized Systems

### build_world_transform_cache (hierarchy.rs)

**Called by:** `render_scene()`, every frame before rendering.

**Strategy:** Extract all entity transforms and relationships into an owned flat data structure, identify root entities, then process each root's subtree in parallel. Each subtree is independent because a child has exactly one parent — no two subtrees share entities.

**Phases:**
1. **Extract:** Query all `(IdComponent, TransformComponent, RelationshipComponent)`, copy into `Vec<EntityData>`. Build `HashMap<u64, usize>` UUID-to-index lookup.
2. **Parallel:** `roots.par_iter()` — each root walks its subtree recursively via `compute_subtree()`, accumulating `(hecs::Entity, Mat4)` results into a local Vec.
3. **Merge:** Combine all sub-Vecs into the final `HashMap<hecs::Entity, Mat4>` cache.

**Fallback:** Below `PAR_THRESHOLD`, delegates to `build_world_transform_cache_sequential()` which uses the original `get_world_transform_cached()` approach (single HashMap, no extraction).

### on_update_animations (rendering.rs)

**Called by:** Game loop, every frame (`scene.on_update_animations(dt)`).

**Strategy:** Extract playing `SpriteAnimatorComponent` state into `AnimWork` structs, tick frame timers in parallel, write results back. All buffers are taken from `RenderBufferPool` (on `SceneCore`), cleared, used, and returned — zero per-frame heap allocations after the first frame.

**Phases:**
1. **Extract:** Query `(Entity, IdComponent, SpriteAnimatorComponent)`, filter to playing animators, copy clip parameters and runtime state into pooled `Vec<AnimWork>`.
2. **Parallel tick:** `par_for_each_mut` advances `frame_timer`, computes `current_frame`, detects clip completion. Pure arithmetic — no World access.
3. **Writeback:** Iterate `AnimWork`, write `frame_timer`, `current_frame`, `playing` back into components. Collect finished events into pooled `Vec`.
4. **Sequential:** `InstancedSpriteAnimator` completion check, Lua `on_animation_finished` callbacks, default clip transitions, controller evaluation. These remain sequential (small N, require `&mut self`).

### render_scene frustum culling (rendering.rs)

**Called by:** `render_scene()`, every frame.

**Strategy:** Extract sprite sort keys, cull AABBs against the view frustum in parallel, sort the final renderable list in parallel. All buffers (`sort_keys`, `sprite_handles`, `circle_keys`) are taken from `RenderBufferPool`, cleared, used, and returned — zero per-frame allocations after warmup.

**Phases:**
1. **Extract:** Query `(Entity, SpriteRendererComponent)`, extend pooled `sprite_handles` Vec.
2. **Parallel cull:** `sprites.par_iter().filter_map()` — each sprite looks up its world transform in the pre-computed cache, builds an AABB, tests against the frustum. Culled sprites are dropped; visible sprites produce sort key tuples extended into pooled `sort_keys`.
3. **Circles/text/tilemaps:** Collected sequentially into pooled buffers (usually few entities).
4. **Parallel sort:** `renderables.par_sort_by()` on `(sorting_layer, order_in_layer, z)`.
5. **Sequential draw:** The actual Vulkan draw calls remain sequential (renderer is `!Send`).

## Profiling Integration

**File:** `profiling.rs`

The profiling system handles worker thread results via a global `WORKER_RESULTS: Mutex<Vec<ProfileResult>>`:

- **Main thread:** `ProfileTimer::stop()` pushes to the thread-local `PROFILE_RESULTS` (existing behavior, zero contention).
- **Worker threads:** Detected by thread name prefix `"gg-worker"`. Push to the global `WORKER_RESULTS` mutex.
- **`drain_profile_results()`:** Drains both the main thread's thread-local and the global worker results, returning a combined Vec.

Chrome Tracing JSON output already works multi-threaded — each thread gets a unique `tid` via `NEXT_THREAD_ID` atomic counter. Worker events appear on separate rows in `chrome://tracing`.

## What Stays Sequential (And Why)

| System | Reason |
|--------|--------|
| `on_update_physics` | rapier2d is `!Send`; interleaves Lua `on_fixed_update` callbacks |
| `on_update_scripts` | `NativeScript::on_update` receives `&mut Scene` |
| `on_update_lua_scripts` | `mlua::Lua` is `!Send` |
| `update_spatial_audio` | `AudioEngine` is `!Send`; usually < 10 sources |
| `render_scene` draw loop | Vulkan command recording is `!Send` |
| `evaluate_animation_controllers` | Reads animator + controller, writes animator — small N |

## Stress Test

The sandbox includes a jobs stress test (`gg_sandbox/src/jobs_stress.rs`) that creates 2000 entities with hierarchy and animators, runs for 300 frames, and captures a Chrome Tracing profile.

```sh
cargo run -p gg_sandbox -- --stress           # debug build
cargo run --release -p gg_sandbox -- --stress  # release build
```

The trace file is written to `target/{debug,release}/gg_jobs_stress.json`. Open in `chrome://tracing` or `edge://tracing`, or analyze with:

```sh
cargo run -p gg_tools -- target/release/gg_jobs_stress.json
```

### Benchmark Results (2000 entities, release build, RTX 4090 / 16 cores)

| Metric | Sequential | Parallel | Notes |
|--------|-----------|----------|-------|
| Avg FPS | 1668 | 1220 | Parallel is slower at 2000 entities |
| `build_world_transform_cache` | 0.201 ms | 0.213 ms | EPW overhead > parallel gain |
| `Scene::render_scene` | 0.272 ms | 0.349 ms | Extraction + thread dispatch cost |

At 2000 entities with lightweight per-entity work (sub-microsecond), the extract-copy-writeback overhead dominates the parallel gains. The parallel paths produce **correct results** and the infrastructure is sound — the crossover point where parallelism pays off is at higher entity counts or heavier per-entity computation.

## Future Work

- **Raise or auto-tune `PAR_THRESHOLD`** based on measured per-item work cost
- **System-level parallelism:** Run non-conflicting systems concurrently (requires `SceneParts` — independently borrowable subsystem decomposition)
- **System scheduler with access declarations:** Auto-detect conflicts, build DAG
- **Unsafe parallel mutation:** Direct archetype-chunk parallel write (skip extract/writeback) for hot inner loops
- **Resources container:** Type-erased singleton state for decoupled system communication
