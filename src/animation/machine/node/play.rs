//! A simplest pose node that extracts pose from a specific animation and prepares it for further use.

use crate::{
    animation::{
        machine::{
            node::{BasePoseNode, EvaluatePose},
            ParameterContainer, PoseNode,
        },
        Animation, AnimationContainer, AnimationPose,
    },
    core::{
        pool::{Handle, Pool},
        reflect::prelude::*,
        visitor::prelude::*,
    },
};
use std::{
    cell::{Ref, RefCell},
    ops::{Deref, DerefMut},
};

/// A simplest pose node that extracts pose from a specific animation and prepares it for further use.
/// Animation handle should point to an animation in some animation container see [`AnimationContainer`] docs
/// for more info.
#[derive(Default, Debug, Visit, Clone, Reflect, PartialEq)]
pub struct PlayAnimation {
    /// Base node.
    pub base: BasePoseNode,

    /// A handle to animation.
    pub animation: Handle<Animation>,

    /// Output pose, it contains a filtered (see [`crate::animation::machine::LayerMask`] for more info) pose from
    /// the animation specified by the `animation` field.
    #[visit(skip)]
    #[reflect(hidden)]
    pub output_pose: RefCell<AnimationPose>,
}

impl Deref for PlayAnimation {
    type Target = BasePoseNode;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for PlayAnimation {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl PlayAnimation {
    /// Creates new PlayAnimation node with given animation handle.
    pub fn new(animation: Handle<Animation>) -> Self {
        Self {
            base: Default::default(),
            animation,
            output_pose: Default::default(),
        }
    }
}

impl EvaluatePose for PlayAnimation {
    fn eval_pose(
        &self,
        _nodes: &Pool<PoseNode>,
        _params: &ParameterContainer,
        animations: &AnimationContainer,
        _dt: f32,
    ) -> Ref<AnimationPose> {
        if let Some(animation) = animations.try_get(self.animation) {
            animation
                .pose()
                .clone_into(&mut self.output_pose.borrow_mut());
        }
        self.output_pose.borrow()
    }

    fn pose(&self) -> Ref<AnimationPose> {
        self.output_pose.borrow()
    }
}
