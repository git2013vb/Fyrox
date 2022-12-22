//! State is a final "container" for animation pose. See [`State`] docs for more info.

use crate::{
    animation::{
        machine::{EvaluatePose, ParameterContainer, PoseNode},
        AnimationContainer, AnimationPose,
    },
    core::{
        algebra::Vector2,
        pool::{Handle, Pool},
        reflect::prelude::*,
        visitor::prelude::*,
    },
    utils::NameProvider,
};
use std::cell::Ref;

/// State is a final "container" for animation pose. It has backing pose node which provides a set of values.
/// States can be connected with each other using _transitions_, states with transitions form a state graph.
#[derive(Default, Debug, Visit, Clone, Reflect, PartialEq)]
pub struct State {
    /// Position of state on the canvas. It is editor-specific data.
    pub position: Vector2<f32>,

    /// Name of the state.
    pub name: String,

    /// Root node of the state that provides the state with animation data.
    #[reflect(read_only)]
    pub root: Handle<PoseNode>,
}

impl NameProvider for State {
    fn name(&self) -> &str {
        &self.name
    }
}

impl State {
    /// Creates new instance of state with a given pose.
    pub fn new(name: &str, root: Handle<PoseNode>) -> Self {
        Self {
            position: Default::default(),
            name: name.to_owned(),
            root,
        }
    }

    /// Returns a final pose of the state.
    pub fn pose<'a>(&self, nodes: &'a Pool<PoseNode>) -> Option<Ref<'a, AnimationPose>> {
        nodes.try_borrow(self.root).map(|root| root.pose())
    }

    pub(super) fn update(
        &mut self,
        nodes: &Pool<PoseNode>,
        params: &ParameterContainer,
        animations: &AnimationContainer,
        dt: f32,
    ) {
        if let Some(root) = nodes.try_borrow(self.root) {
            root.eval_pose(nodes, params, animations, dt);
        }
    }
}
