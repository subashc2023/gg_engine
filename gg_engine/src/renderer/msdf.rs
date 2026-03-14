//! Pure Rust MSDF (Multi-channel Signed Distance Field) generator.
//!
//! Implements the core algorithm from Chlumsky's paper:
//! 1. Parse glyph outlines into edge segments (line, quadratic, cubic)
//! 2. Assign colors to edges so corners have different-colored adjacent edges
//! 3. For each pixel, compute signed distance per color channel
//! 4. Output RGB distances normalized to [0,1]

use std::f64::consts::PI;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
#[repr(u8)]
enum EdgeColor {
    Red = 1,
    Green = 2,
    Blue = 4,
    Cyan = 6,    // Green | Blue
    Magenta = 5, // Red | Blue
    Yellow = 3,  // Red | Green
    White = 7,   // Red | Green | Blue
}

impl EdgeColor {
    fn has_red(self) -> bool {
        (self as u8) & 1 != 0
    }
    fn has_green(self) -> bool {
        (self as u8) & 2 != 0
    }
    fn has_blue(self) -> bool {
        (self as u8) & 4 != 0
    }
}

#[derive(Clone, Copy)]
pub(super) struct Vec2 {
    pub(super) x: f64,
    pub(super) y: f64,
}

impl Vec2 {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y
    }
    fn cross(self, other: Self) -> f64 {
        self.x * other.y - self.y * other.x
    }
    fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    fn sub(self, other: Self) -> Self {
        Self::new(self.x - other.x, self.y - other.y)
    }
    fn add(self, other: Self) -> Self {
        Self::new(self.x + other.x, self.y + other.y)
    }
    fn scale(self, s: f64) -> Self {
        Self::new(self.x * s, self.y * s)
    }
    fn normalize(self) -> Self {
        let len = self.length();
        if len < 1e-12 {
            Self::new(0.0, 0.0)
        } else {
            Self::new(self.x / len, self.y / len)
        }
    }
}

/// A signed distance result: distance value and the orthogonality
/// (dot product between edge direction and point-to-edge direction).
#[derive(Clone, Copy)]
struct SignedDistance {
    dist: f64,
    /// Orthogonality — used as tiebreaker when distances are equal.
    dot: f64,
}

impl SignedDistance {
    const INFINITE: Self = Self {
        dist: f64::MAX,
        dot: 0.0,
    };

    fn abs_less(self, other: Self) -> bool {
        let a = self.dist.abs();
        let b = other.dist.abs();
        if (a - b).abs() < 1e-12 {
            self.dot < other.dot
        } else {
            a < b
        }
    }
}

#[derive(Clone)]
enum EdgeSegment {
    Linear {
        p0: Vec2,
        p1: Vec2,
        color: EdgeColor,
    },
    Quadratic {
        p0: Vec2,
        p1: Vec2,
        p2: Vec2,
        color: EdgeColor,
    },
    Cubic {
        p0: Vec2,
        p1: Vec2,
        p2: Vec2,
        p3: Vec2,
        color: EdgeColor,
    },
}

impl EdgeSegment {
    fn color(&self) -> EdgeColor {
        match self {
            Self::Linear { color, .. } => *color,
            Self::Quadratic { color, .. } => *color,
            Self::Cubic { color, .. } => *color,
        }
    }

    fn set_color(&mut self, c: EdgeColor) {
        match self {
            Self::Linear { color, .. } => *color = c,
            Self::Quadratic { color, .. } => *color = c,
            Self::Cubic { color, .. } => *color = c,
        }
    }

    fn direction_at_start(&self) -> Vec2 {
        match self {
            Self::Linear { p0, p1, .. } => p1.sub(*p0),
            Self::Quadratic { p0, p1, p2, .. } => {
                let d = p1.sub(*p0);
                if d.length() < 1e-12 {
                    p2.sub(*p0)
                } else {
                    d
                }
            }
            Self::Cubic { p0, p1, p2, p3, .. } => {
                let d = p1.sub(*p0);
                if d.length() < 1e-12 {
                    let d2 = p2.sub(*p0);
                    if d2.length() < 1e-12 {
                        p3.sub(*p0)
                    } else {
                        d2
                    }
                } else {
                    d
                }
            }
        }
    }

    fn direction_at_end(&self) -> Vec2 {
        match self {
            Self::Linear { p0, p1, .. } => p1.sub(*p0),
            Self::Quadratic { p0, p1, p2, .. } => {
                let d = p2.sub(*p1);
                if d.length() < 1e-12 {
                    p2.sub(*p0)
                } else {
                    d
                }
            }
            Self::Cubic { p0, p1, p2, p3, .. } => {
                let d = p3.sub(*p2);
                if d.length() < 1e-12 {
                    let d2 = p3.sub(*p1);
                    if d2.length() < 1e-12 {
                        p3.sub(*p0)
                    } else {
                        d2
                    }
                } else {
                    d
                }
            }
        }
    }

    fn signed_distance(&self, p: Vec2) -> SignedDistance {
        match self {
            Self::Linear { p0, p1, .. } => signed_distance_linear(*p0, *p1, p),
            Self::Quadratic { p0, p1, p2, .. } => signed_distance_quadratic(*p0, *p1, *p2, p),
            Self::Cubic { p0, p1, p2, p3, .. } => signed_distance_cubic(*p0, *p1, *p2, *p3, p),
        }
    }
}

struct Contour {
    edges: Vec<EdgeSegment>,
}

pub(super) struct Shape {
    contours: Vec<Contour>,
}

// ---------------------------------------------------------------------------
// Outline builder — ttf_parser → Shape
// ---------------------------------------------------------------------------

pub(super) struct OutlineBuilder {
    contours: Vec<Contour>,
    current_edges: Vec<EdgeSegment>,
    current_pos: Vec2,
    contour_start: Vec2,
}

impl OutlineBuilder {
    pub(super) fn new() -> Self {
        Self {
            contours: Vec::new(),
            current_edges: Vec::new(),
            current_pos: Vec2::new(0.0, 0.0),
            contour_start: Vec2::new(0.0, 0.0),
        }
    }

    pub(super) fn build(self) -> Shape {
        Shape {
            contours: self.contours,
        }
    }
}

impl ttf_parser::OutlineBuilder for OutlineBuilder {
    fn move_to(&mut self, x: f32, y: f32) {
        // Close previous contour if it has edges.
        if !self.current_edges.is_empty() {
            self.contours.push(Contour {
                edges: std::mem::take(&mut self.current_edges),
            });
        }
        self.current_pos = Vec2::new(x as f64, y as f64);
        self.contour_start = self.current_pos;
    }

    fn line_to(&mut self, x: f32, y: f32) {
        let p1 = Vec2::new(x as f64, y as f64);
        // Skip degenerate edges.
        if self.current_pos.sub(p1).length() > 1e-12 {
            self.current_edges.push(EdgeSegment::Linear {
                p0: self.current_pos,
                p1,
                color: EdgeColor::White,
            });
        }
        self.current_pos = p1;
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let cp = Vec2::new(x1 as f64, y1 as f64);
        let p2 = Vec2::new(x as f64, y as f64);
        self.current_edges.push(EdgeSegment::Quadratic {
            p0: self.current_pos,
            p1: cp,
            p2,
            color: EdgeColor::White,
        });
        self.current_pos = p2;
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let cp1 = Vec2::new(x1 as f64, y1 as f64);
        let cp2 = Vec2::new(x2 as f64, y2 as f64);
        let p3 = Vec2::new(x as f64, y as f64);
        self.current_edges.push(EdgeSegment::Cubic {
            p0: self.current_pos,
            p1: cp1,
            p2: cp2,
            p3,
            color: EdgeColor::White,
        });
        self.current_pos = p3;
    }

    fn close(&mut self) {
        // Add closing segment if needed.
        if self.current_pos.sub(self.contour_start).length() > 1e-12
            && !self.current_edges.is_empty()
        {
            self.current_edges.push(EdgeSegment::Linear {
                p0: self.current_pos,
                p1: self.contour_start,
                color: EdgeColor::White,
            });
        }
        self.current_pos = self.contour_start;
        if !self.current_edges.is_empty() {
            self.contours.push(Contour {
                edges: std::mem::take(&mut self.current_edges),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Signed distance computations
// ---------------------------------------------------------------------------

fn signed_distance_linear(p0: Vec2, p1: Vec2, p: Vec2) -> SignedDistance {
    let ab = p1.sub(p0);
    let ap = p.sub(p0);
    let t = ap.dot(ab) / ab.dot(ab);
    let t = t.clamp(0.0, 1.0);

    let closest = p0.add(ab.scale(t));
    let diff = p.sub(closest);
    let dist = diff.length();

    // Sign: positive if point is to the left of the edge direction.
    let sign = if ab.cross(ap) >= 0.0 { 1.0 } else { -1.0 };

    // Orthogonality for tiebreaking.
    let edge_dir = ab.normalize();
    let to_point = diff.normalize();
    let dot = (to_point.dot(edge_dir)).abs();

    SignedDistance {
        dist: sign * dist,
        dot: 1.0 - dot,
    }
}

fn signed_distance_quadratic(p0: Vec2, p1: Vec2, p2: Vec2, p: Vec2) -> SignedDistance {
    // Find closest point on quadratic bezier B(t) = (1-t)^2*p0 + 2t(1-t)*p1 + t^2*p2
    // by sampling + Newton refinement.
    let samples = 8;
    let mut best_t = 0.0f64;
    let mut best_dist_sq = f64::MAX;

    for i in 0..=samples {
        let t = i as f64 / samples as f64;
        let pt = eval_quadratic(p0, p1, p2, t);
        let d = p.sub(pt).dot(p.sub(pt));
        if d < best_dist_sq {
            best_dist_sq = d;
            best_t = t;
        }
    }

    // Newton refinement.
    for _ in 0..4 {
        let pt = eval_quadratic(p0, p1, p2, best_t);
        let dt = eval_quadratic_deriv(p0, p1, p2, best_t);
        let ddt = eval_quadratic_second_deriv(p0, p1, p2);

        let diff = pt.sub(p);
        let num = diff.dot(dt);
        let den = dt.dot(dt) + diff.dot(ddt);
        if den.abs() > 1e-12 {
            best_t -= num / den;
        }
        best_t = best_t.clamp(0.0, 1.0);
    }

    // Also check endpoints.
    let d_start = p.sub(p0).dot(p.sub(p0));
    let d_end = p.sub(p2).dot(p.sub(p2));
    let d_best = p.sub(eval_quadratic(p0, p1, p2, best_t));
    let d_best_sq = d_best.dot(d_best);

    if d_start < d_best_sq && d_start < d_end {
        best_t = 0.0;
    } else if d_end < d_best_sq {
        best_t = 1.0;
    }

    let closest = eval_quadratic(p0, p1, p2, best_t);
    let tangent = eval_quadratic_deriv(p0, p1, p2, best_t);
    let diff = p.sub(closest);
    let dist = diff.length();

    let sign = if tangent.cross(diff) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let dot = {
        let tn = tangent.normalize();
        let dn = diff.normalize();
        (tn.dot(dn)).abs()
    };

    SignedDistance {
        dist: sign * dist,
        dot: 1.0 - dot,
    }
}

fn signed_distance_cubic(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, p: Vec2) -> SignedDistance {
    // Sample + Newton refinement for cubic bezier.
    let samples = 12;
    let mut best_t = 0.0f64;
    let mut best_dist_sq = f64::MAX;

    for i in 0..=samples {
        let t = i as f64 / samples as f64;
        let pt = eval_cubic(p0, p1, p2, p3, t);
        let d = p.sub(pt).dot(p.sub(pt));
        if d < best_dist_sq {
            best_dist_sq = d;
            best_t = t;
        }
    }

    // Newton refinement.
    for _ in 0..5 {
        let pt = eval_cubic(p0, p1, p2, p3, best_t);
        let dt = eval_cubic_deriv(p0, p1, p2, p3, best_t);
        let ddt = eval_cubic_second_deriv(p0, p1, p2, p3, best_t);

        let diff = pt.sub(p);
        let num = diff.dot(dt);
        let den = dt.dot(dt) + diff.dot(ddt);
        if den.abs() > 1e-12 {
            best_t -= num / den;
        }
        best_t = best_t.clamp(0.0, 1.0);
    }

    // Check endpoints.
    let d_start = p.sub(p0).dot(p.sub(p0));
    let d_end = p.sub(p3).dot(p.sub(p3));
    let d_best = p.sub(eval_cubic(p0, p1, p2, p3, best_t));
    let d_best_sq = d_best.dot(d_best);

    if d_start < d_best_sq && d_start < d_end {
        best_t = 0.0;
    } else if d_end < d_best_sq {
        best_t = 1.0;
    }

    let closest = eval_cubic(p0, p1, p2, p3, best_t);
    let tangent = eval_cubic_deriv(p0, p1, p2, p3, best_t);
    let diff = p.sub(closest);
    let dist = diff.length();

    let sign = if tangent.cross(diff) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let dot = {
        let tn = tangent.normalize();
        let dn = diff.normalize();
        (tn.dot(dn)).abs()
    };

    SignedDistance {
        dist: sign * dist,
        dot: 1.0 - dot,
    }
}

// ---------------------------------------------------------------------------
// Bezier evaluation helpers
// ---------------------------------------------------------------------------

fn eval_quadratic(p0: Vec2, p1: Vec2, p2: Vec2, t: f64) -> Vec2 {
    let s = 1.0 - t;
    p0.scale(s * s)
        .add(p1.scale(2.0 * s * t))
        .add(p2.scale(t * t))
}

fn eval_quadratic_deriv(p0: Vec2, p1: Vec2, p2: Vec2, t: f64) -> Vec2 {
    let s = 1.0 - t;
    p1.sub(p0).scale(2.0 * s).add(p2.sub(p1).scale(2.0 * t))
}

fn eval_quadratic_second_deriv(p0: Vec2, p1: Vec2, p2: Vec2) -> Vec2 {
    p0.sub(p1.scale(2.0)).add(p2).scale(2.0)
}

fn eval_cubic(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f64) -> Vec2 {
    let s = 1.0 - t;
    p0.scale(s * s * s)
        .add(p1.scale(3.0 * s * s * t))
        .add(p2.scale(3.0 * s * t * t))
        .add(p3.scale(t * t * t))
}

fn eval_cubic_deriv(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f64) -> Vec2 {
    let s = 1.0 - t;
    p1.sub(p0)
        .scale(3.0 * s * s)
        .add(p2.sub(p1).scale(6.0 * s * t))
        .add(p3.sub(p2).scale(3.0 * t * t))
}

fn eval_cubic_second_deriv(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f64) -> Vec2 {
    let s = 1.0 - t;
    p2.sub(p1.scale(2.0))
        .add(p0)
        .scale(6.0 * s)
        .add(p3.sub(p2.scale(2.0)).add(p1).scale(6.0 * t))
}

// ---------------------------------------------------------------------------
// Edge coloring (Chlumsky's "simple" algorithm)
// ---------------------------------------------------------------------------

const CORNER_ANGLE_THRESHOLD: f64 = 30.0; // degrees — angles sharper than this are corners

fn edge_coloring_simple(shape: &mut Shape, angle_threshold_deg: f64) {
    let threshold_cos = (angle_threshold_deg * PI / 180.0).cos();

    for contour in &mut shape.contours {
        let n = contour.edges.len();
        if n == 0 {
            continue;
        }

        if n == 1 {
            // Single edge: split into 3 parts conceptually — just color it white.
            // With a single edge the median trick still works fine.
            contour.edges[0].set_color(EdgeColor::White);
            continue;
        }

        // Find corners: where the angle between consecutive edge directions is sharp.
        let mut corners = Vec::new();
        for i in 0..n {
            let prev = if i == 0 { n - 1 } else { i - 1 };
            let dir_out = contour.edges[i].direction_at_start().normalize();
            let dir_in = contour.edges[prev].direction_at_end().normalize();
            // Corner if the cross product indicates a sharp turn.
            let cross = dir_in.cross(dir_out);
            let dot = dir_in.dot(dir_out);
            if cross.abs() > (1.0 - threshold_cos) || dot < threshold_cos {
                corners.push(i);
            }
        }

        if corners.is_empty() {
            // Smooth contour: alternate colors.
            let colors = [EdgeColor::Cyan, EdgeColor::Magenta, EdgeColor::Yellow];
            for (i, edge) in contour.edges.iter_mut().enumerate() {
                edge.set_color(colors[i % 3]);
            }
        } else if corners.len() == 1 {
            // Single corner: color edges on each side differently.
            let corner = corners[0];
            let colors = [EdgeColor::Magenta, EdgeColor::Cyan, EdgeColor::Yellow];
            for i in 0..n {
                let idx = (i + n - corner) % n;
                contour.edges[(corner + i) % n].set_color(colors[idx.min(2)]);
            }
        } else {
            // Multiple corners: cycle colors, switching at each corner.
            let colors = [EdgeColor::Cyan, EdgeColor::Magenta, EdgeColor::Yellow];
            // Start from the first corner.
            for (color_idx, ci) in (0..corners.len()).enumerate() {
                let next_corner = corners[(ci + 1) % corners.len()];
                let start = corners[ci];

                // Determine how many edges until next corner.
                let count = if next_corner > start {
                    next_corner - start
                } else {
                    n - start + next_corner
                };

                for j in 0..count {
                    let edge_idx = (start + j) % n;
                    contour.edges[edge_idx].set_color(colors[color_idx % 3]);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Winding number (point-in-shape test for sign correction)
// ---------------------------------------------------------------------------

fn compute_winding_number(shape: &Shape, p: Vec2) -> i32 {
    let mut winding = 0i32;
    for contour in &shape.contours {
        for edge in &contour.edges {
            winding += edge_winding(edge, p);
        }
    }
    winding
}

fn edge_winding(edge: &EdgeSegment, p: Vec2) -> i32 {
    // Count crossings of a horizontal ray from p going right.
    match edge {
        EdgeSegment::Linear { p0, p1, .. } => line_winding(*p0, *p1, p),
        EdgeSegment::Quadratic { p0, p1, p2, .. } => {
            // Subdivide and count as line segments.
            let steps = 8;
            let mut w = 0;
            let mut prev = *p0;
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let cur = eval_quadratic(*p0, *p1, *p2, t);
                w += line_winding(prev, cur, p);
                prev = cur;
            }
            w
        }
        EdgeSegment::Cubic { p0, p1, p2, p3, .. } => {
            let steps = 12;
            let mut w = 0;
            let mut prev = *p0;
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                let cur = eval_cubic(*p0, *p1, *p2, *p3, t);
                w += line_winding(prev, cur, p);
                prev = cur;
            }
            w
        }
    }
}

fn line_winding(a: Vec2, b: Vec2, p: Vec2) -> i32 {
    if a.y <= p.y {
        if b.y > p.y {
            // Upward crossing.
            if (b.sub(a)).cross(p.sub(a)) > 0.0 {
                return 1;
            }
        }
    } else if b.y <= p.y {
        // Downward crossing.
        if (b.sub(a)).cross(p.sub(a)) < 0.0 {
            return -1;
        }
    }
    0
}

// ---------------------------------------------------------------------------
// MSDF generation
// ---------------------------------------------------------------------------

/// Generate an MSDF bitmap for a glyph shape.
///
/// Returns an RGBA8 pixel buffer of `width * height * 4` bytes.
/// The bitmap coordinate system: (0,0) = top-left in the output,
/// but internally we compute in font units and flip Y for Vulkan.
///
/// `px_range` is the distance field range in pixels.
/// `scale` maps font units to pixels.
/// `translate` shifts the shape so it's centered in the bitmap.
pub(super) fn generate_msdf(
    shape: &mut Shape,
    width: u32,
    height: u32,
    px_range: f64,
    scale: Vec2,
    translate: Vec2,
) -> Vec<u8> {
    // Edge coloring.
    edge_coloring_simple(shape, CORNER_ANGLE_THRESHOLD);

    let range_in_shape = Vec2::new(
        if scale.x.abs() > 1e-12 {
            px_range / scale.x
        } else {
            0.0
        },
        if scale.y.abs() > 1e-12 {
            px_range / scale.y
        } else {
            0.0
        },
    );
    // Use the average for normalization.
    let range_norm = (range_in_shape.x + range_in_shape.y) * 0.5;

    let mut pixels = vec![0u8; (width * height * 4) as usize];

    for py in 0..height {
        for px in 0..width {
            // Convert pixel center to shape coordinates.
            // Y-flip: output row 0 = top of glyph = high Y in font coords.
            let shape_x = ((px as f64 + 0.5) - translate.x) / scale.x;
            let shape_y = (((height - 1 - py) as f64 + 0.5) - translate.y) / scale.y;
            let p = Vec2::new(shape_x, shape_y);

            let mut min_dist_r = SignedDistance::INFINITE;
            let mut min_dist_g = SignedDistance::INFINITE;
            let mut min_dist_b = SignedDistance::INFINITE;

            for contour in &shape.contours {
                for edge in &contour.edges {
                    let sd = edge.signed_distance(p);
                    let color = edge.color();
                    if color.has_red() && sd.abs_less(min_dist_r) {
                        min_dist_r = sd;
                    }
                    if color.has_green() && sd.abs_less(min_dist_g) {
                        min_dist_g = sd;
                    }
                    if color.has_blue() && sd.abs_less(min_dist_b) {
                        min_dist_b = sd;
                    }
                }
            }

            // Sign correction via winding number.
            let winding = compute_winding_number(shape, p);
            let inside = winding != 0;

            let correct = |sd: SignedDistance| -> f64 {
                let mut d = sd.dist;
                // If the sign disagrees with the winding rule, flip it.
                if inside && d < 0.0 {
                    d = d.abs();
                } else if !inside && d > 0.0 {
                    d = -d.abs();
                }
                d
            };

            let dr = correct(min_dist_r);
            let dg = correct(min_dist_g);
            let db = correct(min_dist_b);

            // Normalize: 0.5 = edge, >0.5 = inside, <0.5 = outside.
            let normalize = |d: f64| -> u8 {
                let v = 0.5 + d / (2.0 * range_norm);
                (v.clamp(0.0, 1.0) * 255.0) as u8
            };

            let offset = ((py * width + px) * 4) as usize;
            pixels[offset] = normalize(dr);
            pixels[offset + 1] = normalize(dg);
            pixels[offset + 2] = normalize(db);
            pixels[offset + 3] = 255;
        }
    }

    pixels
}

/// Compute the bounding box of a shape in font units.
/// Returns (min_x, min_y, max_x, max_y).
pub(super) fn shape_bounds(shape: &Shape) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = f64::MIN;
    let mut max_y = f64::MIN;
    let mut has_points = false;

    for contour in &shape.contours {
        for edge in &contour.edges {
            let points = edge_sample_points(edge);
            for p in &points {
                has_points = true;
                min_x = min_x.min(p.x);
                min_y = min_y.min(p.y);
                max_x = max_x.max(p.x);
                max_y = max_y.max(p.y);
            }
        }
    }

    if has_points {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

fn edge_sample_points(edge: &EdgeSegment) -> Vec<Vec2> {
    match edge {
        EdgeSegment::Linear { p0, p1, .. } => vec![*p0, *p1],
        EdgeSegment::Quadratic { p0, p1, p2, .. } => {
            let mut pts = Vec::with_capacity(9);
            for i in 0..=8 {
                let t = i as f64 / 8.0;
                pts.push(eval_quadratic(*p0, *p1, *p2, t));
            }
            pts
        }
        EdgeSegment::Cubic { p0, p1, p2, p3, .. } => {
            let mut pts = Vec::with_capacity(13);
            for i in 0..=12 {
                let t = i as f64 / 12.0;
                pts.push(eval_cubic(*p0, *p1, *p2, *p3, t));
            }
            pts
        }
    }
}

/// Autoframe: compute scale and translate to fit a shape into a bitmap
/// with the given pixel range for the distance field.
///
/// Returns (scale, translate) or None if the shape has no bounds.
pub(super) fn autoframe(
    shape: &Shape,
    width: u32,
    height: u32,
    px_range: f64,
) -> Option<(Vec2, Vec2)> {
    let (min_x, min_y, max_x, max_y) = shape_bounds(shape)?;

    let shape_w = max_x - min_x;
    let shape_h = max_y - min_y;

    if shape_w < 1e-12 || shape_h < 1e-12 {
        return None;
    }

    // Available pixels for the actual glyph (minus range padding on each side).
    let avail_w = width as f64 - 2.0 * px_range;
    let avail_h = height as f64 - 2.0 * px_range;

    if avail_w <= 0.0 || avail_h <= 0.0 {
        return None;
    }

    // Uniform scale to preserve glyph aspect ratio.
    let scale_val = (avail_w / shape_w).min(avail_h / shape_h);
    let scale = Vec2::new(scale_val, scale_val);

    // Center the glyph in the available area.
    let used_w = shape_w * scale_val;
    let used_h = shape_h * scale_val;
    let tx = px_range + (avail_w - used_w) * 0.5 - min_x * scale_val;
    let ty = px_range + (avail_h - used_h) * 0.5 - min_y * scale_val;
    let translate = Vec2::new(tx, ty);

    Some((scale, translate))
}
