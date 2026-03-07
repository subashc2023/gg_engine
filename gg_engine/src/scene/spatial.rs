//! Spatial optimization utilities: AABB, frustum culling, and uniform spatial grid.
//!
//! Provides 2D axis-aligned bounding box tests and a sparse uniform grid for
//! efficient region queries. Used by the rendering pipeline for frustum culling
//! and available to game logic for spatial queries (e.g. "find entities near X").

use std::collections::HashMap;

/// Axis-aligned bounding box in 2D world space.
#[derive(Clone, Copy, Debug, Default)]
pub struct Aabb2D {
    pub min: glam::Vec2,
    pub max: glam::Vec2,
}

impl Aabb2D {
    #[inline]
    pub fn new(min: glam::Vec2, max: glam::Vec2) -> Self {
        Self { min, max }
    }

    /// Test if two AABBs overlap (inclusive on all edges).
    #[inline]
    pub fn overlaps(&self, other: &Aabb2D) -> bool {
        self.min.x <= other.max.x
            && self.max.x >= other.min.x
            && self.min.y <= other.max.y
            && self.max.y >= other.min.y
    }

    /// Compute the AABB of a unit quad (`[-0.5, 0.5]²`) under the given 4×4
    /// world transform.
    ///
    /// Works for any combination of translation, rotation, and non-uniform scale.
    /// Uses the absolute values of the transform's 2D basis vectors to compute
    /// the tightest axis-aligned bounds.
    #[inline]
    pub fn from_unit_quad_transform(m: &glam::Mat4) -> Self {
        let cx = m.w_axis.x;
        let cy = m.w_axis.y;
        // For a unit quad, each basis contributes ±0.5 to the extent.
        let hx = (m.x_axis.x.abs() + m.y_axis.x.abs()) * 0.5;
        let hy = (m.x_axis.y.abs() + m.y_axis.y.abs()) * 0.5;
        Self {
            min: glam::Vec2::new(cx - hx, cy - hy),
            max: glam::Vec2::new(cx + hx, cy + hy),
        }
    }

    /// Returns true if both min and max are finite (not NaN or Inf).
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.min.is_finite() && self.max.is_finite()
    }

    /// Expand the AABB by `margin` in all directions.
    #[inline]
    pub fn expand(&self, margin: f32) -> Self {
        Self {
            min: self.min - glam::Vec2::splat(margin),
            max: self.max + glam::Vec2::splat(margin),
        }
    }

    /// Returns true if the point is inside the AABB (inclusive).
    #[inline]
    pub fn contains_point(&self, point: glam::Vec2) -> bool {
        point.x >= self.min.x
            && point.x <= self.max.x
            && point.y >= self.min.y
            && point.y <= self.max.y
    }
}

/// Extract the camera's visible 2D AABB from the inverse view-projection matrix.
///
/// For each NDC corner, un-projects a ray from the near plane (`z=0`) to the
/// far plane (`z=1`) and intersects it with the world-space `z=0` plane where
/// 2D entities live. This handles both orthographic cameras (ray has constant
/// x,y) and perspective cameras (ray expands with distance) correctly.
///
/// **Note**: This can degenerate when a perspective camera is tilted nearly
/// parallel to the z=0 plane. For robust entity-level frustum culling, prefer
/// [`Frustum2D`] which uses half-plane tests instead.
#[allow(dead_code)]
pub fn camera_frustum_aabb(vp_inv: &glam::Mat4) -> Aabb2D {
    let mut min = glam::Vec2::splat(f32::INFINITY);
    let mut max = glam::Vec2::splat(f32::NEG_INFINITY);
    for &(nx, ny) in &[(-1.0f32, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        let near = vp_inv.project_point3(glam::Vec3::new(nx, ny, 0.0));
        let far = vp_inv.project_point3(glam::Vec3::new(nx, ny, 1.0));
        let dz = far.z - near.z;
        let p = if dz.abs() > 1e-6 {
            // Perspective: intersect the frustum edge ray with the z=0 world plane.
            let t = -near.z / dz;
            glam::Vec2::new(
                near.x + t * (far.x - near.x),
                near.y + t * (far.y - near.y),
            )
        } else {
            // Orthographic: near and far have identical x,y — use directly.
            near.truncate()
        };
        if p.is_finite() {
            min = min.min(p);
            max = max.max(p);
        }
    }
    Aabb2D { min, max }
}

/// Frustum culling via half-plane tests in 2D (z=0 world plane).
///
/// Extracts the four side planes (left, right, bottom, top) from a
/// view-projection matrix using the Gribb/Hartmann method. Since all
/// 2D entities live at z=0, each 3D plane equation `ax + by + cz + d >= 0`
/// reduces to the 2D half-plane `ax + by + d >= 0`.
///
/// This is robust for any camera orientation (orthographic, perspective,
/// tilted, orbited) and never degenerates the way ray-plane intersection does.
pub struct Frustum2D {
    /// Four 2D half-planes: `(a, b, d)` where `ax + by + d >= 0` means inside.
    /// Order: left, right, bottom, top.
    planes: [(f32, f32, f32); 4],
}

impl Frustum2D {
    /// Extract 2D frustum planes from a view-projection matrix.
    ///
    /// The VP matrix should include any Y-flip (e.g. Vulkan) — the plane
    /// extraction accounts for it automatically.
    pub fn from_view_projection(vp: &glam::Mat4) -> Self {
        // Gribb/Hartmann frustum plane extraction from column-major VP.
        // Row i of the matrix = (col0[i], col1[i], col2[i], col3[i]).
        // Each plane (a, b, c, d) satisfies: ax + by + cz + d >= 0 inside.
        // For z=0 entities, c drops out -> half-plane (a, b, d).
        let c0 = vp.x_axis;
        let c1 = vp.y_axis;
        let c3 = vp.w_axis;

        // Left:   cx + cw >= 0  ->  row3 + row0
        let left = (c0.w + c0.x, c1.w + c1.x, c3.w + c3.x);
        // Right:  cw - cx >= 0  ->  row3 - row0
        let right = (c0.w - c0.x, c1.w - c1.x, c3.w - c3.x);
        // Bottom: cy + cw >= 0  ->  row3 + row1
        let bottom = (c0.w + c0.y, c1.w + c1.y, c3.w + c3.y);
        // Top:    cw - cy >= 0  ->  row3 - row1
        let top = (c0.w - c0.y, c1.w - c1.y, c3.w - c3.y);

        Self {
            planes: [left, right, bottom, top],
        }
    }

    /// Test whether a 2D AABB (at z=0) is at least partially inside the frustum.
    ///
    /// Returns `false` if the AABB is fully outside any frustum plane (culled).
    /// Uses the p-vertex test: for each plane, the corner most in the direction
    /// of the plane normal is checked. If that corner is outside, the entire
    /// AABB is outside this plane.
    #[inline]
    pub fn contains_aabb(&self, aabb: &Aabb2D) -> bool {
        for &(a, b, d) in &self.planes {
            let px = if a >= 0.0 { aabb.max.x } else { aabb.min.x };
            let py = if b >= 0.0 { aabb.max.y } else { aabb.min.y };
            if a * px + b * py + d < 0.0 {
                return false;
            }
        }
        true
    }
}

/// Uniform spatial grid for efficient 2D region queries.
///
/// Divides world space into cells of fixed size. Each entity is inserted into
/// every cell its AABB overlaps. Queries return all entities in cells that
/// overlap the query region (may include false positives that the caller can
/// refine with exact tests).
pub struct SpatialGrid {
    cell_size: f32,
    inv_cell_size: f32,
    cells: HashMap<(i32, i32), Vec<hecs::Entity>>,
    entity_count: usize,
}

impl SpatialGrid {
    /// Create an empty spatial grid with the given cell size in world units.
    pub fn new(cell_size: f32) -> Self {
        let cell_size = cell_size.max(0.01);
        Self {
            cell_size,
            inv_cell_size: 1.0 / cell_size,
            cells: HashMap::new(),
            entity_count: 0,
        }
    }

    /// Insert an entity into all grid cells overlapped by its AABB.
    pub fn insert(&mut self, entity: hecs::Entity, aabb: &Aabb2D) {
        let (min_cx, min_cy) = self.cell_coords(aabb.min);
        let (max_cx, max_cy) = self.cell_coords(aabb.max);
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                self.cells.entry((cx, cy)).or_default().push(entity);
            }
        }
        self.entity_count += 1;
    }

    /// Query all entities whose cells overlap the given AABB region.
    ///
    /// May contain duplicates (an entity spanning multiple cells appears once
    /// per overlapping cell). Use [`query_region_dedup`](Self::query_region_dedup)
    /// for unique results.
    pub fn query_region(&self, region: &Aabb2D) -> Vec<hecs::Entity> {
        let (min_cx, min_cy) = self.cell_coords(region.min);
        let (max_cx, max_cy) = self.cell_coords(region.max);
        let mut result = Vec::new();
        for cy in min_cy..=max_cy {
            for cx in min_cx..=max_cx {
                if let Some(entities) = self.cells.get(&(cx, cy)) {
                    result.extend_from_slice(entities);
                }
            }
        }
        result
    }

    /// Query all unique entities whose cells overlap the given AABB region.
    pub fn query_region_dedup(&self, region: &Aabb2D) -> Vec<hecs::Entity> {
        let mut result = self.query_region(region);
        result.sort_unstable_by_key(|e| e.id());
        result.dedup();
        result
    }

    /// Number of occupied cells.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// Number of entities inserted.
    pub fn entity_count(&self) -> usize {
        self.entity_count
    }

    /// The cell size used by this grid.
    pub fn cell_size(&self) -> f32 {
        self.cell_size
    }

    /// Convert a world-space position to grid cell coordinates.
    #[inline]
    fn cell_coords(&self, pos: glam::Vec2) -> (i32, i32) {
        (
            (pos.x * self.inv_cell_size).floor() as i32,
            (pos.y * self.inv_cell_size).floor() as i32,
        )
    }
}

// -------------------------------------------------------------------------
// Tests
// -------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_overlap_partial() {
        let a = Aabb2D::new(glam::Vec2::ZERO, glam::Vec2::ONE);
        let b = Aabb2D::new(glam::Vec2::splat(0.5), glam::Vec2::splat(1.5));
        assert!(a.overlaps(&b));
        assert!(b.overlaps(&a));
    }

    #[test]
    fn aabb_no_overlap() {
        let a = Aabb2D::new(glam::Vec2::ZERO, glam::Vec2::ONE);
        let b = Aabb2D::new(glam::Vec2::splat(2.0), glam::Vec2::splat(3.0));
        assert!(!a.overlaps(&b));
        assert!(!b.overlaps(&a));
    }

    #[test]
    fn aabb_edge_touching_is_overlap() {
        let a = Aabb2D::new(glam::Vec2::ZERO, glam::Vec2::ONE);
        let b = Aabb2D::new(glam::Vec2::new(1.0, 0.0), glam::Vec2::new(2.0, 1.0));
        assert!(a.overlaps(&b));
    }

    #[test]
    fn aabb_from_identity_transform() {
        let aabb = Aabb2D::from_unit_quad_transform(&glam::Mat4::IDENTITY);
        assert!((aabb.min.x - (-0.5)).abs() < 1e-5);
        assert!((aabb.min.y - (-0.5)).abs() < 1e-5);
        assert!((aabb.max.x - 0.5).abs() < 1e-5);
        assert!((aabb.max.y - 0.5).abs() < 1e-5);
    }

    #[test]
    fn aabb_from_scaled_transform() {
        let m = glam::Mat4::from_scale(glam::Vec3::new(4.0, 2.0, 1.0));
        let aabb = Aabb2D::from_unit_quad_transform(&m);
        assert!((aabb.min.x - (-2.0)).abs() < 1e-5);
        assert!((aabb.min.y - (-1.0)).abs() < 1e-5);
        assert!((aabb.max.x - 2.0).abs() < 1e-5);
        assert!((aabb.max.y - 1.0).abs() < 1e-5);
    }

    #[test]
    fn aabb_from_translated_transform() {
        let m = glam::Mat4::from_translation(glam::Vec3::new(10.0, 20.0, 0.0));
        let aabb = Aabb2D::from_unit_quad_transform(&m);
        assert!((aabb.min.x - 9.5).abs() < 1e-5);
        assert!((aabb.min.y - 19.5).abs() < 1e-5);
        assert!((aabb.max.x - 10.5).abs() < 1e-5);
        assert!((aabb.max.y - 20.5).abs() < 1e-5);
    }

    #[test]
    fn aabb_from_rotated_transform() {
        // 45-degree rotation: unit quad becomes a diamond.
        let m = glam::Mat4::from_rotation_z(std::f32::consts::FRAC_PI_4);
        let aabb = Aabb2D::from_unit_quad_transform(&m);
        let expected = std::f32::consts::FRAC_1_SQRT_2;
        assert!((aabb.min.x - (-expected)).abs() < 1e-4);
        assert!((aabb.max.x - expected).abs() < 1e-4);
        assert!((aabb.min.y - (-expected)).abs() < 1e-4);
        assert!((aabb.max.y - expected).abs() < 1e-4);
    }

    #[test]
    fn aabb_from_scale_rotation_translation() {
        // Scale 2x, rotate 90°, translate to (5, 3).
        let m = glam::Mat4::from_scale_rotation_translation(
            glam::Vec3::new(2.0, 2.0, 1.0),
            glam::Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            glam::Vec3::new(5.0, 3.0, 0.0),
        );
        let aabb = Aabb2D::from_unit_quad_transform(&m);
        // 2x scaled unit quad rotated 90° is still a 2x2 square.
        assert!((aabb.min.x - 4.0).abs() < 1e-4);
        assert!((aabb.max.x - 6.0).abs() < 1e-4);
        assert!((aabb.min.y - 2.0).abs() < 1e-4);
        assert!((aabb.max.y - 4.0).abs() < 1e-4);
    }

    #[test]
    fn camera_frustum_orthographic() {
        // Orthographic VP: maps world [-10,10] x [-5,5] to NDC [-1,1].
        // Vulkan Y-flip: proj.y_axis.y *= -1.
        let mut proj = glam::Mat4::orthographic_lh(-10.0, 10.0, -5.0, 5.0, -1.0, 1.0);
        proj.y_axis.y *= -1.0; // Vulkan Y-flip
        let vp_inv = proj.inverse();
        let aabb = camera_frustum_aabb(&vp_inv);
        assert!(aabb.is_valid());
        assert!((aabb.min.x - (-10.0)).abs() < 1e-3, "min.x={}", aabb.min.x);
        assert!((aabb.max.x - 10.0).abs() < 1e-3, "max.x={}", aabb.max.x);
        assert!((aabb.min.y - (-5.0)).abs() < 1e-3, "min.y={}", aabb.min.y);
        assert!((aabb.max.y - 5.0).abs() < 1e-3, "max.y={}", aabb.max.y);
    }

    #[test]
    fn camera_frustum_perspective() {
        // Perspective camera at z=10 looking at z=0.
        let mut proj = glam::Mat4::perspective_lh(
            std::f32::consts::FRAC_PI_4, // 45 degree FOV
            16.0 / 9.0,
            0.01,
            1000.0,
        );
        proj.y_axis.y *= -1.0; // Vulkan Y-flip
        let view = glam::Mat4::look_at_lh(
            glam::Vec3::new(0.0, 0.0, 10.0), // camera at z=10
            glam::Vec3::ZERO,                 // looking at origin
            glam::Vec3::Y,
        );
        let vp = proj * view;
        let vp_inv = vp.inverse();
        let aabb = camera_frustum_aabb(&vp_inv);
        assert!(aabb.is_valid(), "AABB should be valid");
        // At z=0 (distance 10 from camera), 45° FOV gives half_height = 10 * tan(22.5°) ≈ 4.14
        let expected_half_h = 10.0 * (std::f32::consts::FRAC_PI_8).tan();
        let expected_half_w = expected_half_h * 16.0 / 9.0;
        assert!(
            (aabb.max.y - expected_half_h).abs() < 0.5,
            "max.y={} expected ~{}",
            aabb.max.y,
            expected_half_h
        );
        assert!(
            (aabb.max.x - expected_half_w).abs() < 0.5,
            "max.x={} expected ~{}",
            aabb.max.x,
            expected_half_w
        );
    }

    #[test]
    fn aabb_expand() {
        let aabb = Aabb2D::new(glam::Vec2::ZERO, glam::Vec2::ONE);
        let expanded = aabb.expand(0.5);
        assert!((expanded.min.x - (-0.5)).abs() < 1e-5);
        assert!((expanded.max.x - 1.5).abs() < 1e-5);
    }

    #[test]
    fn aabb_contains_point() {
        let aabb = Aabb2D::new(glam::Vec2::ZERO, glam::Vec2::ONE);
        assert!(aabb.contains_point(glam::Vec2::splat(0.5)));
        assert!(!aabb.contains_point(glam::Vec2::splat(2.0)));
        // Edge is inclusive.
        assert!(aabb.contains_point(glam::Vec2::ZERO));
    }

    #[test]
    fn spatial_grid_cell_coords() {
        let grid = SpatialGrid::new(10.0);
        assert_eq!(grid.cell_coords(glam::Vec2::new(5.0, 5.0)), (0, 0));
        assert_eq!(grid.cell_coords(glam::Vec2::new(15.0, 15.0)), (1, 1));
        assert_eq!(grid.cell_coords(glam::Vec2::new(-5.0, -5.0)), (-1, -1));
        assert_eq!(grid.cell_coords(glam::Vec2::new(0.0, 0.0)), (0, 0));
        assert_eq!(grid.cell_coords(glam::Vec2::new(-0.01, 0.0)), (-1, 0));
    }

    #[test]
    fn spatial_grid_insert_and_query() {
        let mut world = hecs::World::new();
        let e1 = world.spawn(());
        let e2 = world.spawn(());
        let e3 = world.spawn(());

        let mut grid = SpatialGrid::new(10.0);
        // e1 in cell (0,0)
        grid.insert(
            e1,
            &Aabb2D::new(glam::Vec2::new(1.0, 1.0), glam::Vec2::new(5.0, 5.0)),
        );
        // e2 far away in cell (2,2)
        grid.insert(
            e2,
            &Aabb2D::new(glam::Vec2::new(20.0, 20.0), glam::Vec2::new(25.0, 25.0)),
        );
        // e3 spans cells (0,0) and (1,0) and (0,1) and (1,1)
        grid.insert(
            e3,
            &Aabb2D::new(glam::Vec2::new(5.0, 5.0), glam::Vec2::new(15.0, 15.0)),
        );

        // Query region around origin should find e1 and e3 but not e2.
        let result = grid.query_region_dedup(&Aabb2D::new(
            glam::Vec2::ZERO,
            glam::Vec2::splat(5.0),
        ));
        assert!(result.contains(&e1));
        assert!(result.contains(&e3));
        assert!(!result.contains(&e2));
    }

    #[test]
    fn spatial_grid_empty_query() {
        let grid = SpatialGrid::new(10.0);
        let result = grid.query_region(&Aabb2D::new(
            glam::Vec2::ZERO,
            glam::Vec2::splat(100.0),
        ));
        assert!(result.is_empty());
    }

    #[test]
    fn spatial_grid_entity_count() {
        let mut world = hecs::World::new();
        let e1 = world.spawn(());
        let e2 = world.spawn(());

        let mut grid = SpatialGrid::new(10.0);
        grid.insert(
            e1,
            &Aabb2D::new(glam::Vec2::ZERO, glam::Vec2::ONE),
        );
        grid.insert(
            e2,
            &Aabb2D::new(glam::Vec2::splat(50.0), glam::Vec2::splat(51.0)),
        );
        assert_eq!(grid.entity_count(), 2);
    }

    // --- Frustum2D tests ---

    #[test]
    fn frustum2d_orthographic() {
        // Orthographic VP: world [-10,10] x [-5,5] with Vulkan Y-flip.
        let mut proj = glam::Mat4::orthographic_lh(-10.0, 10.0, -5.0, 5.0, -1.0, 1.0);
        proj.y_axis.y *= -1.0;
        let frustum = Frustum2D::from_view_projection(&proj);

        // Inside the frustum.
        let inside = Aabb2D::new(glam::Vec2::new(-1.0, -1.0), glam::Vec2::new(1.0, 1.0));
        assert!(frustum.contains_aabb(&inside));

        // Outside to the right.
        let right = Aabb2D::new(glam::Vec2::new(11.0, -1.0), glam::Vec2::new(12.0, 1.0));
        assert!(!frustum.contains_aabb(&right));

        // Outside to the left.
        let left = Aabb2D::new(glam::Vec2::new(-12.0, -1.0), glam::Vec2::new(-11.0, 1.0));
        assert!(!frustum.contains_aabb(&left));

        // Outside above.
        let above = Aabb2D::new(glam::Vec2::new(-1.0, 6.0), glam::Vec2::new(1.0, 7.0));
        assert!(!frustum.contains_aabb(&above));

        // Outside below.
        let below = Aabb2D::new(glam::Vec2::new(-1.0, -7.0), glam::Vec2::new(1.0, -6.0));
        assert!(!frustum.contains_aabb(&below));

        // Partially overlapping (should be kept).
        let partial = Aabb2D::new(glam::Vec2::new(9.0, -1.0), glam::Vec2::new(11.0, 1.0));
        assert!(frustum.contains_aabb(&partial));
    }

    #[test]
    fn frustum2d_perspective_straight_down() {
        // Perspective camera at z=10 looking straight down at z=0.
        let mut proj = glam::Mat4::perspective_lh(
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            0.01,
            1000.0,
        );
        proj.y_axis.y *= -1.0;
        let view = glam::Mat4::look_at_lh(
            glam::Vec3::new(0.0, 0.0, 10.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let vp = proj * view;
        let frustum = Frustum2D::from_view_projection(&vp);

        // Entity at the origin should be visible.
        let center = Aabb2D::new(glam::Vec2::new(-0.5, -0.5), glam::Vec2::new(0.5, 0.5));
        assert!(frustum.contains_aabb(&center));

        // Entity far away should be culled.
        let far_away = Aabb2D::new(glam::Vec2::new(100.0, 100.0), glam::Vec2::new(101.0, 101.0));
        assert!(!frustum.contains_aabb(&far_away));
    }

    #[test]
    fn frustum2d_perspective_tilted() {
        // Perspective camera tilted 45 degrees — should NOT degenerate.
        let mut proj = glam::Mat4::perspective_lh(
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            0.01,
            1000.0,
        );
        proj.y_axis.y *= -1.0;
        // Camera at (0, 10, 10) looking at origin — 45° tilt.
        let view = glam::Mat4::look_at_lh(
            glam::Vec3::new(0.0, 10.0, 10.0),
            glam::Vec3::ZERO,
            glam::Vec3::Y,
        );
        let vp = proj * view;
        let frustum = Frustum2D::from_view_projection(&vp);

        // Entity at origin should still be visible (camera looks at it).
        let center = Aabb2D::new(glam::Vec2::new(-0.5, -0.5), glam::Vec2::new(0.5, 0.5));
        assert!(frustum.contains_aabb(&center));

        // Entity far behind the camera should be culled.
        let behind = Aabb2D::new(glam::Vec2::new(-0.5, 50.0), glam::Vec2::new(0.5, 51.0));
        assert!(!frustum.contains_aabb(&behind));
    }

    #[test]
    fn frustum2d_perspective_steep_tilt() {
        // Camera nearly parallel to z=0 — the old AABB approach would degenerate.
        let mut proj = glam::Mat4::perspective_lh(
            std::f32::consts::FRAC_PI_4,
            16.0 / 9.0,
            0.01,
            1000.0,
        );
        proj.y_axis.y *= -1.0;
        // Camera at (0, 0.1, 5) looking at (0, 0, -10) — nearly parallel to z=0.
        let view = glam::Mat4::look_at_lh(
            glam::Vec3::new(0.0, 0.1, 5.0),
            glam::Vec3::new(0.0, 0.0, -10.0),
            glam::Vec3::Y,
        );
        let vp = proj * view;
        let frustum = Frustum2D::from_view_projection(&vp);

        // The frustum should still function — no panics, no degenerate behavior.
        // Entity near the look-at line should be visible.
        let near_target = Aabb2D::new(glam::Vec2::new(-1.0, -1.0), glam::Vec2::new(1.0, 1.0));
        assert!(frustum.contains_aabb(&near_target));

        // Entity far to the side should be culled.
        let far_side = Aabb2D::new(glam::Vec2::new(100.0, 0.0), glam::Vec2::new(101.0, 1.0));
        assert!(!frustum.contains_aabb(&far_side));
    }
}
