//! Node is a part of animation blending tree, that backs a state with animation data. See [`PoseNode`] docs for
//! more info.

use crate::{
    animation::{
        machine::{
            node::{blend::BlendAnimations, play::PlayAnimation},
            BlendAnimationsByIndex, BlendPose, IndexedBlendInput, ParameterContainer, State,
        },
        Animation, AnimationContainer, AnimationPose,
    },
    core::{
        algebra::Vector2,
        pool::{Handle, Pool},
        reflect::prelude::*,
        visitor::prelude::*,
    },
};
use std::{
    cell::Ref,
    ops::{Deref, DerefMut},
};

pub mod blend;
pub mod play;

/// A set of common data fields that is used in every node.
#[derive(Debug, Visit, Clone, Default, Reflect, PartialEq)]
pub struct BasePoseNode {
    /// Position on the canvas, it is editor-specific data.
    pub position: Vector2<f32>,

    /// A handle of parent state that "owns" the node.
    #[reflect(hidden)]
    pub parent_state: Handle<State>,
}

/// Specialized node that provides animation pose. See documentation for each variant.
#[derive(Debug, Visit, Clone, Reflect, PartialEq)]
pub enum PoseNode {
    /// See docs for [`PlayAnimation`].
    PlayAnimation(PlayAnimation),

    /// See docs for [`BlendAnimations`].
    BlendAnimations(BlendAnimations),

    /// See docs for [`BlendAnimationsByIndex`].
    BlendAnimationsByIndex(BlendAnimationsByIndex),
}

impl Default for PoseNode {
    fn default() -> Self {
        Self::PlayAnimation(Default::default())
    }
}

impl PoseNode {
    /// Creates new node that plays an animation.
    pub fn make_play_animation(animation: Handle<Animation>) -> Self {
        Self::PlayAnimation(PlayAnimation::new(animation))
    }

    /// Creates new node that blends multiple poses into one.
    pub fn make_blend_animations(poses: Vec<BlendPose>) -> Self {
        Self::BlendAnimations(BlendAnimations::new(poses))
    }

    /// Creates new node that switches between given animations using index and smoothly blends from
    /// one animation to another while switching.
    pub fn make_blend_animations_by_index(
        index_parameter: String,
        inputs: Vec<IndexedBlendInput>,
    ) -> Self {
        Self::BlendAnimationsByIndex(BlendAnimationsByIndex::new(index_parameter, inputs))
    }

    /// Returns a set of handles to children pose nodes.
    pub fn children(&self) -> Vec<Handle<PoseNode>> {
        match self {
            Self::PlayAnimation(_) => {
                // No children nodes.
                vec![]
            }
            Self::BlendAnimations(definition) => definition.children(),
            Self::BlendAnimationsByIndex(definition) => definition.children(),
        }
    }
}

macro_rules! static_dispatch {
    ($self:ident, $func:ident, $($args:expr),*) => {
        match $self {
            PoseNode::PlayAnimation(v) => v.$func($($args),*),
            PoseNode::BlendAnimations(v) => v.$func($($args),*),
            PoseNode::BlendAnimationsByIndex(v) => v.$func($($args),*),
        }
    };
}

impl Deref for PoseNode {
    type Target = BasePoseNode;

    fn deref(&self) -> &Self::Target {
        static_dispatch!(self, deref,)
    }
}

impl DerefMut for PoseNode {
    fn deref_mut(&mut self) -> &mut Self::Target {
        static_dispatch!(self, deref_mut,)
    }
}

/// A trait that responsible for animation pose evaluation.
pub trait EvaluatePose {
    /// Evaluates animation pose and returns a reference to it.
    fn eval_pose(
        &self,
        nodes: &Pool<PoseNode>,
        params: &ParameterContainer,
        animations: &AnimationContainer,
        dt: f32,
    ) -> Ref<AnimationPose>;

    /// Returns a reference to inner pose of a node.
    fn pose(&self) -> Ref<AnimationPose>;
}

impl EvaluatePose for PoseNode {
    fn eval_pose(
        &self,
        nodes: &Pool<PoseNode>,
        params: &ParameterContainer,
        animations: &AnimationContainer,
        dt: f32,
    ) -> Ref<AnimationPose> {
        static_dispatch!(self, eval_pose, nodes, params, animations, dt)
    }

    fn pose(&self) -> Ref<AnimationPose> {
        static_dispatch!(self, pose,)
    }
}
