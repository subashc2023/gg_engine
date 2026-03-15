use glam::{Mat4, Quat, Vec3};

/// Maximum number of bone matrices that can be submitted per frame across all
/// skinned meshes. 4096 × 64 bytes = 256 KB in the SSBO.
pub const MAX_SKINNED_BONES_PER_FRAME: usize = 4096;

// ---------------------------------------------------------------------------
// Skeleton
// ---------------------------------------------------------------------------

/// Joint hierarchy and inverse bind matrices for a skeletal mesh.
///
/// The skeleton is shared (via `Arc`) across all entities that use the same
/// model. Per-entity state (current clip, playback time) lives on the
/// component.
#[derive(Clone)]
pub struct Skeleton {
    /// Human-readable joint names (indexed by joint index).
    pub joint_names: Vec<String>,
    /// Parent index for each joint. `-1` means root (no parent).
    pub parent_indices: Vec<i32>,
    /// Inverse bind matrices: transform from mesh space to bone-local space.
    pub inverse_bind_matrices: Vec<Mat4>,
    /// Rest pose local transforms from the glTF node hierarchy.
    /// Used as defaults for joints/channels without animation data.
    pub rest_local_transforms: Vec<Mat4>,
    /// Correction matrix: `inverse(meshNodeGlobal) × rootJointAncestorGlobal`.
    /// Accounts for the mesh node's position in the glTF scene graph relative
    /// to the skeleton root. Applied as a prefix to all bone matrices per the
    /// glTF spec: `jointMatrix = inverse(meshGlobal) × jointGlobal × IBM`.
    pub bind_space_correction: Mat4,
}

impl Skeleton {
    /// Number of joints in the skeleton.
    pub fn joint_count(&self) -> usize {
        self.joint_names.len()
    }

    /// Evaluate a pose at the given `time` within an animation clip.
    ///
    /// Returns a [`BonePose`] with one matrix per joint:
    /// `final[j] = world_transform[j] × inverse_bind_matrix[j]`
    pub fn compute_pose(&self, clip: &SkeletalAnimationClip, time: f32) -> BonePose {
        let num_joints = self.joint_count();
        // Start from rest pose — joints without animation channels keep their
        // bind-time local transforms instead of collapsing to identity.
        let mut local_transforms = self.rest_local_transforms.clone();

        // Sample each channel's TRS at the given time.
        // Only override components that have keyframes; missing components
        // retain the rest pose value.
        for channel in &clip.channels {
            let j = channel.joint_index;
            if j >= num_joints {
                continue;
            }
            let (rest_scale, rest_rotation, rest_translation) =
                self.rest_local_transforms[j].to_scale_rotation_translation();

            let translation = if channel.translations.is_empty() {
                rest_translation
            } else {
                sample_vec3(&channel.translations, time)
            };
            let rotation = if channel.rotations.is_empty() {
                rest_rotation
            } else {
                sample_quat(&channel.rotations, time)
            };
            let scale = if channel.scales.is_empty() {
                rest_scale
            } else {
                sample_vec3_or(&channel.scales, time, Vec3::ONE)
            };

            local_transforms[j] =
                Mat4::from_scale_rotation_translation(scale, rotation, translation);
        }

        // Forward kinematics: propagate parent transforms.
        let mut world_transforms = vec![Mat4::IDENTITY; num_joints];
        for j in 0..num_joints {
            let parent = self.parent_indices[j];
            if parent >= 0 && (parent as usize) < num_joints {
                world_transforms[j] = world_transforms[parent as usize] * local_transforms[j];
            } else {
                world_transforms[j] = local_transforms[j];
            }
        }

        // Multiply by inverse bind matrix and apply bind-space correction
        // per the glTF spec: jointMatrix = inverse(meshGlobal) × jointGlobal × IBM.
        let matrices: Vec<Mat4> = (0..num_joints)
            .map(|j| {
                self.bind_space_correction * world_transforms[j] * self.inverse_bind_matrices[j]
            })
            .collect();

        BonePose { matrices }
    }

    /// Compute the bind pose (all joints at their rest position).
    /// The result should be near-identity if the rest pose matches the
    /// inverse bind matrices (i.e. the model appears in its T-pose).
    pub fn bind_pose(&self) -> BonePose {
        let num_joints = self.joint_count();
        let mut world_transforms = vec![Mat4::IDENTITY; num_joints];
        for j in 0..num_joints {
            let parent = self.parent_indices[j];
            if parent >= 0 && (parent as usize) < num_joints {
                world_transforms[j] =
                    world_transforms[parent as usize] * self.rest_local_transforms[j];
            } else {
                world_transforms[j] = self.rest_local_transforms[j];
            }
        }
        let matrices = (0..num_joints)
            .map(|j| {
                self.bind_space_correction * world_transforms[j] * self.inverse_bind_matrices[j]
            })
            .collect();
        BonePose { matrices }
    }
}

// ---------------------------------------------------------------------------
// Animation data
// ---------------------------------------------------------------------------

/// A keyframe sample at a specific time.
#[derive(Clone)]
pub struct Keyframe<T: Clone> {
    pub time: f32,
    pub value: T,
}

/// Animation data for a single joint: separate TRS channels.
#[derive(Clone)]
pub struct JointChannel {
    pub joint_index: usize,
    pub translations: Vec<Keyframe<Vec3>>,
    pub rotations: Vec<Keyframe<Quat>>,
    pub scales: Vec<Keyframe<Vec3>>,
}

/// A named event marker at a specific time in a skeletal animation clip.
///
/// When playback crosses this time, the engine fires an
/// `on_animation_event(event_name, clip_name)` Lua callback on the entity.
#[derive(Clone, Debug)]
pub struct SkeletalAnimationEvent {
    /// Time in seconds within the clip that triggers this event.
    pub time: f32,
    /// The event name passed to the Lua callback (e.g. "footstep", "attack_hit").
    pub name: String,
}

/// A complete skeletal animation clip (e.g. "Walk", "Run", "Idle").
#[derive(Clone)]
pub struct SkeletalAnimationClip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<JointChannel>,
}

// ---------------------------------------------------------------------------
// BonePose — computed per-frame per-entity
// ---------------------------------------------------------------------------

/// Computed bone matrices for a single frame.
pub struct BonePose {
    /// Final matrices: `joint_world[j] × inverse_bind[j]` for each joint.
    pub matrices: Vec<Mat4>,
}

impl BonePose {
    /// Linearly interpolate between two poses per-bone.
    ///
    /// `t = 0.0` returns `a`, `t = 1.0` returns `b`.
    /// Works well for short crossfade durations (0.1–0.5 s) where the minor
    /// volume distortion from matrix lerp is imperceptible.
    pub fn blend(a: &BonePose, b: &BonePose, t: f32) -> BonePose {
        let t = t.clamp(0.0, 1.0);
        let len = a.matrices.len().min(b.matrices.len());
        let matrices = (0..len)
            .map(|i| {
                let ma = a.matrices[i].to_cols_array();
                let mb = b.matrices[i].to_cols_array();
                let mut out = [0.0f32; 16];
                for k in 0..16 {
                    out[k] = ma[k] + (mb[k] - ma[k]) * t;
                }
                Mat4::from_cols_array(&out)
            })
            .collect();
        BonePose { matrices }
    }
}

// ---------------------------------------------------------------------------
// Keyframe sampling (linear interpolation)
// ---------------------------------------------------------------------------

fn sample_vec3(keyframes: &[Keyframe<Vec3>], time: f32) -> Vec3 {
    sample_vec3_or(keyframes, time, Vec3::ZERO)
}

fn sample_vec3_or(keyframes: &[Keyframe<Vec3>], time: f32, default: Vec3) -> Vec3 {
    if keyframes.is_empty() {
        return default;
    }
    if keyframes.len() == 1 || time <= keyframes[0].time {
        return keyframes[0].value;
    }
    let last = keyframes.last().unwrap();
    if time >= last.time {
        return last.value;
    }
    // Binary search for the surrounding keyframes.
    let idx = keyframes
        .partition_point(|kf| kf.time <= time)
        .saturating_sub(1);
    let a = &keyframes[idx];
    let b = &keyframes[(idx + 1).min(keyframes.len() - 1)];
    let span = b.time - a.time;
    if span <= 0.0 {
        return a.value;
    }
    let t = (time - a.time) / span;
    a.value.lerp(b.value, t)
}

fn sample_quat(keyframes: &[Keyframe<Quat>], time: f32) -> Quat {
    if keyframes.is_empty() {
        return Quat::IDENTITY;
    }
    if keyframes.len() == 1 || time <= keyframes[0].time {
        return keyframes[0].value;
    }
    let last = keyframes.last().unwrap();
    if time >= last.time {
        return last.value;
    }
    let idx = keyframes
        .partition_point(|kf| kf.time <= time)
        .saturating_sub(1);
    let a = &keyframes[idx];
    let b = &keyframes[(idx + 1).min(keyframes.len() - 1)];
    let span = b.time - a.time;
    if span <= 0.0 {
        return a.value;
    }
    let t = (time - a.time) / span;
    a.value.slerp(b.value, t)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_vec3_single() {
        let kf = vec![Keyframe {
            time: 0.0,
            value: Vec3::ONE,
        }];
        assert_eq!(sample_vec3(&kf, 0.0), Vec3::ONE);
        assert_eq!(sample_vec3(&kf, 1.0), Vec3::ONE);
    }

    #[test]
    fn sample_vec3_lerp() {
        let kf = vec![
            Keyframe {
                time: 0.0,
                value: Vec3::ZERO,
            },
            Keyframe {
                time: 1.0,
                value: Vec3::ONE,
            },
        ];
        let mid = sample_vec3(&kf, 0.5);
        assert!((mid - Vec3::splat(0.5)).length() < 0.001);
    }

    #[test]
    fn sample_quat_endpoints() {
        let kf = vec![
            Keyframe {
                time: 0.0,
                value: Quat::IDENTITY,
            },
            Keyframe {
                time: 1.0,
                value: Quat::from_rotation_y(std::f32::consts::PI),
            },
        ];
        let start = sample_quat(&kf, 0.0);
        assert!(start.abs_diff_eq(Quat::IDENTITY, 0.001));
    }

    #[test]
    fn bind_pose_is_identity() {
        let skeleton = Skeleton {
            joint_names: vec!["root".into(), "child".into()],
            parent_indices: vec![-1, 0],
            inverse_bind_matrices: vec![Mat4::IDENTITY; 2],
            rest_local_transforms: vec![Mat4::IDENTITY; 2],
            bind_space_correction: Mat4::IDENTITY,
        };
        let pose = skeleton.bind_pose();
        assert_eq!(pose.matrices.len(), 2);
        for m in &pose.matrices {
            assert!((*m - Mat4::IDENTITY).abs_diff_eq(Mat4::ZERO, 0.001));
        }
    }

    #[test]
    fn forward_kinematics_chain() {
        // Two-joint chain: root translates (1,0,0), child translates (0,1,0).
        // World transform of child = root * child = translate(1,1,0).
        let skeleton = Skeleton {
            joint_names: vec!["root".into(), "child".into()],
            parent_indices: vec![-1, 0],
            inverse_bind_matrices: vec![Mat4::IDENTITY; 2],
            rest_local_transforms: vec![Mat4::IDENTITY; 2],
            bind_space_correction: Mat4::IDENTITY,
        };
        let clip = SkeletalAnimationClip {
            name: "test".into(),
            duration: 1.0,
            channels: vec![
                JointChannel {
                    joint_index: 0,
                    translations: vec![Keyframe {
                        time: 0.0,
                        value: Vec3::new(1.0, 0.0, 0.0),
                    }],
                    rotations: vec![],
                    scales: vec![],
                },
                JointChannel {
                    joint_index: 1,
                    translations: vec![Keyframe {
                        time: 0.0,
                        value: Vec3::new(0.0, 1.0, 0.0),
                    }],
                    rotations: vec![],
                    scales: vec![],
                },
            ],
        };
        let pose = skeleton.compute_pose(&clip, 0.0);
        // Root bone: translate(1,0,0) * I = translate(1,0,0)
        let root_pos = pose.matrices[0].w_axis.truncate();
        assert!((root_pos - Vec3::new(1.0, 0.0, 0.0)).length() < 0.001);
        // Child bone: translate(1,0,0) * translate(0,1,0) * I = translate(1,1,0)
        let child_pos = pose.matrices[1].w_axis.truncate();
        assert!((child_pos - Vec3::new(1.0, 1.0, 0.0)).length() < 0.001);
    }

    #[test]
    fn blend_pose_endpoints() {
        let a = BonePose {
            matrices: vec![Mat4::IDENTITY],
        };
        let b = BonePose {
            matrices: vec![Mat4::from_translation(Vec3::new(2.0, 0.0, 0.0))],
        };
        // t=0 → a
        let p0 = BonePose::blend(&a, &b, 0.0);
        assert!((p0.matrices[0] - Mat4::IDENTITY).abs_diff_eq(Mat4::ZERO, 0.001));
        // t=1 → b
        let p1 = BonePose::blend(&a, &b, 1.0);
        let pos = p1.matrices[0].w_axis.truncate();
        assert!((pos - Vec3::new(2.0, 0.0, 0.0)).length() < 0.001);
    }

    #[test]
    fn blend_pose_midpoint() {
        let a = BonePose {
            matrices: vec![Mat4::from_translation(Vec3::ZERO)],
        };
        let b = BonePose {
            matrices: vec![Mat4::from_translation(Vec3::new(4.0, 0.0, 0.0))],
        };
        let mid = BonePose::blend(&a, &b, 0.5);
        let pos = mid.matrices[0].w_axis.truncate();
        assert!((pos - Vec3::new(2.0, 0.0, 0.0)).length() < 0.001);
    }

    #[test]
    fn blend_pose_clamps_t() {
        let a = BonePose {
            matrices: vec![Mat4::IDENTITY],
        };
        let b = BonePose {
            matrices: vec![Mat4::from_translation(Vec3::X)],
        };
        // t < 0 clamped to 0
        let p = BonePose::blend(&a, &b, -1.0);
        assert!((p.matrices[0] - Mat4::IDENTITY).abs_diff_eq(Mat4::ZERO, 0.001));
        // t > 1 clamped to 1
        let p = BonePose::blend(&a, &b, 5.0);
        let pos = p.matrices[0].w_axis.truncate();
        assert!((pos - Vec3::X).length() < 0.001);
    }

    #[test]
    fn blend_pose_different_lengths() {
        // Shorter array determines output length.
        let a = BonePose {
            matrices: vec![Mat4::IDENTITY; 3],
        };
        let b = BonePose {
            matrices: vec![Mat4::IDENTITY; 2],
        };
        let result = BonePose::blend(&a, &b, 0.5);
        assert_eq!(result.matrices.len(), 2);
    }
}
