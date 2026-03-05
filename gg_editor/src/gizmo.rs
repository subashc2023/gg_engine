use gg_engine::prelude::*;
use serde::{Deserialize, Serialize};
use transform_gizmo_egui::{EnumSet, GizmoMode};

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub(crate) enum GizmoOperation {
    None,      // Q — select mode, no gizmo
    #[serde(alias = "translate")]
    #[default]
    Translate, // W
    Rotate,    // E
    Scale,     // R
}

pub(crate) fn gizmo_modes_for(op: GizmoOperation) -> EnumSet<GizmoMode> {
    match op {
        GizmoOperation::None => EnumSet::empty(),
        GizmoOperation::Translate => {
            GizmoMode::TranslateX
                | GizmoMode::TranslateY
                | GizmoMode::TranslateZ
                | GizmoMode::TranslateXY
                | GizmoMode::TranslateXZ
                | GizmoMode::TranslateYZ
        }
        GizmoOperation::Rotate => GizmoMode::RotateX | GizmoMode::RotateY | GizmoMode::RotateZ,
        GizmoOperation::Scale => {
            GizmoMode::ScaleX | GizmoMode::ScaleY | GizmoMode::ScaleZ | GizmoMode::ScaleUniform
        }
    }
}

/// Convert a glam Mat4 (f32) to a row-major f64 array for the gizmo library.
///
/// GizmoConfig stores matrices as `mint::RowMatrix4<f64>`.  The `From<[[f64;4];4]>`
/// impl for RowMatrix4 treats the outer arrays as **rows**, so we must supply
/// rows, not columns.  `transpose().to_cols_array_2d()` gives us exactly that
/// (columns of M^T = rows of M).
pub(crate) fn mat4_to_f64(m: &Mat4) -> [[f64; 4]; 4] {
    m.transpose()
        .to_cols_array_2d()
        .map(|row| row.map(|v| v as f64))
}
