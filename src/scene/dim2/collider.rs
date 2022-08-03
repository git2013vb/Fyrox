//! Collider is a geometric entity that can be attached to a rigid body to allow participate it
//! participate in contact generation, collision response and proximity queries.

use crate::{
    core::{
        algebra::Vector2,
        inspect::{Inspect, PropertyInfo},
        math::aabb::AxisAlignedBoundingBox,
        pool::Handle,
        reflect::Reflect,
        uuid::{uuid, Uuid},
        variable::{InheritError, InheritableVariable, TemplateVariable},
        visitor::prelude::*,
    },
    engine::resource_manager::ResourceManager,
    impl_directly_inheritable_entity_trait,
    scene::{
        base::{Base, BaseBuilder},
        collider::InteractionGroups,
        dim2::physics::{ContactPair, PhysicsWorld},
        graph::{map::NodeHandleMap, physics::CoefficientCombineRule, Graph},
        node::{Node, NodeTrait, SyncContext, TypeUuidProvider},
        DirectlyInheritableEntity,
    },
    utils::log::Log,
};
use rapier2d::geometry::ColliderHandle;
use std::{
    cell::Cell,
    ops::{Deref, DerefMut},
};
use strum_macros::{AsRefStr, EnumString, EnumVariantNames};

/// Ball is an idea sphere shape defined by a single parameters - its radius.
#[derive(Clone, Debug, Visit, PartialEq, Inspect, Reflect)]
pub struct BallShape {
    /// Radius of the sphere.
    #[inspect(min_value = 0.0, step = 0.05)]
    pub radius: f32,
}

impl Default for BallShape {
    fn default() -> Self {
        Self { radius: 0.5 }
    }
}

/// Cuboid shape (rectangle).
#[derive(Clone, Debug, Visit, PartialEq, Inspect, Reflect)]
pub struct CuboidShape {
    /// Half extents of the box. X - half width, Y - half height.
    /// Actual _size_ will be 2 times bigger.
    pub half_extents: Vector2<f32>,
}

impl Default for CuboidShape {
    fn default() -> Self {
        Self {
            half_extents: Vector2::new(0.5, 0.5),
        }
    }
}

/// Arbitrary capsule shape defined by 2 points (which forms axis) and a radius.
#[derive(Clone, Debug, Visit, PartialEq, Inspect, Reflect)]
pub struct CapsuleShape {
    /// Begin point of the capsule.
    pub begin: Vector2<f32>,
    /// End point of the capsule.
    pub end: Vector2<f32>,
    /// Radius of the capsule.
    #[inspect(min_value = 0.0, step = 0.05)]
    pub radius: f32,
}

impl Default for CapsuleShape {
    // Y-capsule
    fn default() -> Self {
        Self {
            begin: Default::default(),
            end: Vector2::new(0.0, 1.0),
            radius: 0.5,
        }
    }
}

/// Arbitrary segment shape defined by two points.
#[derive(Clone, Debug, Visit, PartialEq, Inspect, Reflect)]
pub struct SegmentShape {
    /// Begin point of the capsule.
    pub begin: Vector2<f32>,
    /// End point of the capsule.
    pub end: Vector2<f32>,
}

impl Default for SegmentShape {
    fn default() -> Self {
        Self {
            begin: Default::default(),
            end: Vector2::new(0.0, 1.0),
        }
    }
}

/// Arbitrary triangle shape.
#[derive(Clone, Debug, Visit, PartialEq, Inspect, Reflect)]
pub struct TriangleShape {
    /// First point of the triangle shape.
    pub a: Vector2<f32>,
    /// Second point of the triangle shape.
    pub b: Vector2<f32>,
    /// Third point of the triangle shape.
    pub c: Vector2<f32>,
}

impl Default for TriangleShape {
    fn default() -> Self {
        Self {
            a: Default::default(),
            b: Vector2::new(1.0, 0.0),
            c: Vector2::new(0.0, 1.0),
        }
    }
}

/// Geometry source for colliders with complex geometry.
///
/// # Notes
///
/// Currently there is only one way to set geometry - using a scene node as a source of data.
#[derive(Default, Clone, Copy, PartialEq, Hash, Debug, Visit, Inspect, Reflect)]
pub struct GeometrySource(pub Handle<Node>);

/// Arbitrary triangle mesh shape.
#[derive(Default, Clone, Debug, PartialEq, Visit, Inspect, Reflect)]
pub struct TrimeshShape {
    /// Geometry sources for the shape.
    pub sources: Vec<GeometrySource>,
}

/// Arbitrary height field shape.
#[derive(Default, Clone, Debug, PartialEq, Visit, Inspect, Reflect)]
pub struct HeightfieldShape {
    /// A handle to terrain scene node.
    pub geometry_source: GeometrySource,
}

impl Inspect for ColliderShape {
    fn properties(&self) -> Vec<PropertyInfo<'_>> {
        match self {
            ColliderShape::Ball(v) => v.properties(),
            ColliderShape::Cuboid(v) => v.properties(),
            ColliderShape::Capsule(v) => v.properties(),
            ColliderShape::Segment(v) => v.properties(),
            ColliderShape::Triangle(v) => v.properties(),
            ColliderShape::Trimesh(v) => v.properties(),
            ColliderShape::Heightfield(v) => v.properties(),
        }
    }
}

/// Possible collider shapes.
#[derive(Clone, Debug, Visit, Reflect, AsRefStr, PartialEq, EnumString, EnumVariantNames)]
pub enum ColliderShape {
    /// See [`BallShape`] docs.
    Ball(BallShape),
    /// See [`CuboidShape`] docs.
    Cuboid(CuboidShape),
    /// See [`CapsuleShape`] docs.
    Capsule(CapsuleShape),
    /// See [`SegmentShape`] docs.
    Segment(SegmentShape),
    /// See [`TriangleShape`] docs.
    Triangle(TriangleShape),
    /// See [`TrimeshShape`] docs.
    Trimesh(TrimeshShape),
    /// See [`HeightfieldShape`] docs.
    Heightfield(HeightfieldShape),
}

impl Default for ColliderShape {
    fn default() -> Self {
        Self::Ball(Default::default())
    }
}

impl ColliderShape {
    /// Initializes a ball shape defined by its radius.
    pub fn ball(radius: f32) -> Self {
        Self::Ball(BallShape { radius })
    }

    /// Initializes a cuboid shape defined by its half-extents.
    pub fn cuboid(hx: f32, hy: f32) -> Self {
        Self::Cuboid(CuboidShape {
            half_extents: Vector2::new(hx, hy),
        })
    }

    /// Initializes a capsule shape from its endpoints and radius.
    pub fn capsule(begin: Vector2<f32>, end: Vector2<f32>, radius: f32) -> Self {
        Self::Capsule(CapsuleShape { begin, end, radius })
    }

    /// Initializes a new collider builder with a capsule shape aligned with the `x` axis.
    pub fn capsule_x(half_height: f32, radius: f32) -> Self {
        let p = Vector2::x() * half_height;
        Self::capsule(-p, p, radius)
    }

    /// Initializes a new collider builder with a capsule shape aligned with the `y` axis.
    pub fn capsule_y(half_height: f32, radius: f32) -> Self {
        let p = Vector2::y() * half_height;
        Self::capsule(-p, p, radius)
    }

    /// Initializes a segment shape from its endpoints.
    pub fn segment(begin: Vector2<f32>, end: Vector2<f32>) -> Self {
        Self::Segment(SegmentShape { begin, end })
    }

    /// Initializes a triangle shape.
    pub fn triangle(a: Vector2<f32>, b: Vector2<f32>, c: Vector2<f32>) -> Self {
        Self::Triangle(TriangleShape { a, b, c })
    }

    /// Initializes a triangle mesh shape defined by a set of handles to mesh nodes that will be
    /// used to create physical shape.
    pub fn trimesh(geometry_sources: Vec<GeometrySource>) -> Self {
        Self::Trimesh(TrimeshShape {
            sources: geometry_sources,
        })
    }

    /// Initializes a heightfield shape defined by a handle to terrain node.
    pub fn heightfield(geometry_source: GeometrySource) -> Self {
        Self::Heightfield(HeightfieldShape { geometry_source })
    }
}

/// Collider is a geometric entity that can be attached to a rigid body to allow participate it
/// participate in contact generation, collision response and proximity queries.
#[derive(Inspect, Reflect, Visit, Debug)]
pub struct Collider {
    base: Base,

    #[inspect(deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_shape")]
    pub(crate) shape: TemplateVariable<ColliderShape>,

    #[inspect(min_value = 0.0, step = 0.05, deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_friction")]
    pub(crate) friction: TemplateVariable<f32>,

    #[inspect(deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_density")]
    pub(crate) density: TemplateVariable<Option<f32>>,

    #[inspect(min_value = 0.0, step = 0.05, deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_restitution")]
    pub(crate) restitution: TemplateVariable<f32>,

    #[inspect(deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_is_sensor")]
    pub(crate) is_sensor: TemplateVariable<bool>,

    #[inspect(deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_collision_groups")]
    pub(crate) collision_groups: TemplateVariable<InteractionGroups>,

    #[inspect(deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_solver_groups")]
    pub(crate) solver_groups: TemplateVariable<InteractionGroups>,

    #[inspect(deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_friction_combine_rule")]
    pub(crate) friction_combine_rule: TemplateVariable<CoefficientCombineRule>,

    #[inspect(deref, is_modified = "is_modified()")]
    #[reflect(deref, setter = "set_restitution_combine_rule")]
    pub(crate) restitution_combine_rule: TemplateVariable<CoefficientCombineRule>,

    #[visit(skip)]
    #[inspect(skip)]
    #[reflect(hidden)]
    pub(crate) native: Cell<ColliderHandle>,
}

impl_directly_inheritable_entity_trait!(Collider;
    shape,
    friction,
    density,
    restitution,
    is_sensor,
    collision_groups,
    solver_groups,
    friction_combine_rule,
    restitution_combine_rule
);

impl Default for Collider {
    fn default() -> Self {
        Self {
            base: Default::default(),
            shape: Default::default(),
            friction: Default::default(),
            density: Default::default(),
            restitution: Default::default(),
            is_sensor: Default::default(),
            collision_groups: Default::default(),
            solver_groups: Default::default(),
            friction_combine_rule: Default::default(),
            restitution_combine_rule: Default::default(),
            native: Cell::new(ColliderHandle::invalid()),
        }
    }
}

impl Deref for Collider {
    type Target = Base;

    fn deref(&self) -> &Self::Target {
        &self.base
    }
}

impl DerefMut for Collider {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.base
    }
}

impl Clone for Collider {
    fn clone(&self) -> Self {
        Self {
            base: self.base.clone(),
            shape: self.shape.clone(),
            friction: self.friction.clone(),
            density: self.density.clone(),
            restitution: self.restitution.clone(),
            is_sensor: self.is_sensor.clone(),
            collision_groups: self.collision_groups.clone(),
            solver_groups: self.solver_groups.clone(),
            friction_combine_rule: self.friction_combine_rule.clone(),
            restitution_combine_rule: self.restitution_combine_rule.clone(),
            // Do not copy.
            native: Cell::new(ColliderHandle::invalid()),
        }
    }
}

impl TypeUuidProvider for Collider {
    fn type_uuid() -> Uuid {
        uuid!("2b1659ea-a116-4224-bcd4-7931e3ae3b40")
    }
}

impl Collider {
    /// Sets the new shape to the collider.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_shape(&mut self, shape: ColliderShape) -> ColliderShape {
        self.shape.set(shape)
    }

    /// Returns shared reference to the collider shape.
    pub fn shape(&self) -> &ColliderShape {
        &self.shape
    }

    /// Returns a copy of the collider shape.
    pub fn shape_value(&self) -> ColliderShape {
        (*self.shape).clone()
    }

    /// Returns mutable reference to the current collider shape.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn shape_mut(&mut self) -> &mut ColliderShape {
        self.shape.get_mut()
    }

    /// Sets the new restitution value. The exact meaning of possible values is somewhat complex,
    /// check [Wikipedia page](https://en.wikipedia.org/wiki/Coefficient_of_restitution) for more
    /// info.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_restitution(&mut self, restitution: f32) -> f32 {
        self.restitution.set(restitution)
    }

    /// Returns current restitution value of the collider.
    pub fn restitution(&self) -> f32 {
        *self.restitution
    }

    /// Sets the new density value of the collider. Density defines actual mass of the rigid body to
    /// which the collider is attached. Final mass will be a sum of `ColliderVolume * ColliderDensity`
    /// of each collider. In case if density is undefined, the mass of the collider will be zero,
    /// which will lead to two possible effects:
    ///
    /// 1) If a rigid body to which collider is attached have no additional mass, then the rigid body
    ///    won't rotate, only move.
    /// 2) If the rigid body have some additional mass, then the rigid body will have normal behaviour.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_density(&mut self, density: Option<f32>) -> Option<f32> {
        self.density.set(density)
    }

    /// Returns current density of the collider.
    pub fn density(&self) -> Option<f32> {
        *self.density
    }

    /// Sets friction coefficient for the collider. The greater value is the more kinematic energy
    /// will be converted to heat (in other words - lost), the parent rigid body will slowdown much
    /// faster and so on.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_friction(&mut self, friction: f32) -> f32 {
        self.friction.set(friction)
    }

    /// Return current friction of the collider.
    pub fn friction(&self) -> f32 {
        *self.friction
    }

    /// Sets the new collision filtering options. See [`InteractionGroups`] docs for more info.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_collision_groups(&mut self, groups: InteractionGroups) -> InteractionGroups {
        self.collision_groups.set(groups)
    }

    /// Returns current collision filtering options.
    pub fn collision_groups(&self) -> InteractionGroups {
        *self.collision_groups
    }

    /// Sets the new joint solver filtering options. See [`InteractionGroups`] docs for more info.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_solver_groups(&mut self, groups: InteractionGroups) -> InteractionGroups {
        self.solver_groups.set(groups)
    }

    /// Returns current solver groups.
    pub fn solver_groups(&self) -> InteractionGroups {
        *self.solver_groups
    }

    /// If true is passed, the method makes collider a sensor. Sensors will not participate in
    /// collision response, but it is still possible to query contact information from them.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_is_sensor(&mut self, is_sensor: bool) -> bool {
        self.is_sensor.set(is_sensor)
    }

    /// Returns true if the collider is sensor, false - otherwise.
    pub fn is_sensor(&self) -> bool {
        *self.is_sensor
    }

    /// Sets the new friction combine rule. See [`CoefficientCombineRule`] docs for more info.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_friction_combine_rule(
        &mut self,
        rule: CoefficientCombineRule,
    ) -> CoefficientCombineRule {
        self.friction_combine_rule.set(rule)
    }

    /// Returns current friction combine rule of the collider.
    pub fn friction_combine_rule(&self) -> CoefficientCombineRule {
        *self.friction_combine_rule
    }

    /// Sets the new restitution combine rule. See [`CoefficientCombineRule`] docs for more info.
    ///
    /// # Performance
    ///
    /// This is relatively expensive operation - it forces the physics engine to recalculate contacts,
    /// perform collision response, etc. Try avoid calling this method each frame for better
    /// performance.
    pub fn set_restitution_combine_rule(
        &mut self,
        rule: CoefficientCombineRule,
    ) -> CoefficientCombineRule {
        self.restitution_combine_rule.set(rule)
    }

    /// Returns current restitution combine rule of the collider.
    pub fn restitution_combine_rule(&self) -> CoefficientCombineRule {
        *self.restitution_combine_rule
    }

    /// Returns an iterator that yields contact information for the collider.
    pub fn contacts<'a>(
        &self,
        physics: &'a PhysicsWorld,
    ) -> impl Iterator<Item = ContactPair> + 'a {
        physics.contacts_with(self.native.get())
    }

    pub(crate) fn needs_sync_model(&self) -> bool {
        self.shape.need_sync()
            || self.friction.need_sync()
            || self.density.need_sync()
            || self.restitution.need_sync()
            || self.is_sensor.need_sync()
            || self.collision_groups.need_sync()
            || self.solver_groups.need_sync()
            || self.friction_combine_rule.need_sync()
            || self.restitution_combine_rule.need_sync()
    }
}

impl NodeTrait for Collider {
    crate::impl_query_component!();

    fn local_bounding_box(&self) -> AxisAlignedBoundingBox {
        self.base.local_bounding_box()
    }

    fn world_bounding_box(&self) -> AxisAlignedBoundingBox {
        self.base.world_bounding_box()
    }

    // Prefab inheritance resolving.
    fn inherit(&mut self, parent: &Node) -> Result<(), InheritError> {
        self.base.inherit_properties(parent)?;
        if let Some(parent) = parent.cast::<Self>() {
            self.try_inherit_self_properties(parent)?;
        }
        Ok(())
    }

    fn reset_inheritable_properties(&mut self) {
        self.base.reset_inheritable_properties();
        self.reset_self_inheritable_properties();
    }

    fn restore_resources(&mut self, resource_manager: ResourceManager) {
        self.base.restore_resources(resource_manager);
    }

    fn remap_handles(&mut self, old_new_mapping: &NodeHandleMap) {
        self.base.remap_handles(old_new_mapping);

        match self.shape.get_mut_silent() {
            ColliderShape::Trimesh(ref mut trimesh) => {
                for source in trimesh.sources.iter_mut() {
                    if !old_new_mapping.try_map(&mut source.0) {
                        Log::warn(format!(
                            "Unable to remap geometry source of a Trimesh collider {} shape. Handle is {}!",
                            *self.base.name,
                            source.0
                        ))
                    }
                }
            }
            ColliderShape::Heightfield(ref mut heightfield) => {
                if !old_new_mapping.try_map(&mut heightfield.geometry_source.0) {
                    Log::warn(format!(
                        "Unable to remap geometry source of a Height Field collider {} shape. Handle is {}!",
                        *self.base.name,
                        heightfield.geometry_source.0
                    ))
                }
            }
            _ => (),
        }
    }

    fn id(&self) -> Uuid {
        Self::type_uuid()
    }

    fn clean_up(&mut self, graph: &mut Graph) {
        graph.physics2d.remove_collider(self.native.get());

        Log::info(format!(
            "Native collider 2D was removed for node: {}",
            self.name()
        ));
    }

    fn sync_native(&self, self_handle: Handle<Node>, context: &mut SyncContext) {
        context
            .physics2d
            .sync_to_collider_node(context.nodes, self_handle, self);
    }
}

/// Collider builder allows you to build a collider node in declarative manner.
pub struct ColliderBuilder {
    base_builder: BaseBuilder,
    shape: ColliderShape,
    friction: f32,
    density: Option<f32>,
    restitution: f32,
    is_sensor: bool,
    collision_groups: InteractionGroups,
    solver_groups: InteractionGroups,
    friction_combine_rule: CoefficientCombineRule,
    restitution_combine_rule: CoefficientCombineRule,
}

impl ColliderBuilder {
    /// Creates new collider builder.
    pub fn new(base_builder: BaseBuilder) -> Self {
        Self {
            base_builder,
            shape: Default::default(),
            friction: 0.0,
            density: None,
            restitution: 0.0,
            is_sensor: false,
            collision_groups: Default::default(),
            solver_groups: Default::default(),
            friction_combine_rule: Default::default(),
            restitution_combine_rule: Default::default(),
        }
    }

    /// Sets desired shape of the collider.
    pub fn with_shape(mut self, shape: ColliderShape) -> Self {
        self.shape = shape;
        self
    }

    /// Sets desired density value.
    pub fn with_density(mut self, density: Option<f32>) -> Self {
        self.density = density;
        self
    }

    /// Sets desired restitution value.
    pub fn with_restitution(mut self, restitution: f32) -> Self {
        self.restitution = restitution;
        self
    }

    /// Sets desired friction value.    
    pub fn with_friction(mut self, friction: f32) -> Self {
        self.friction = friction;
        self
    }

    /// Sets whether this collider will be used a sensor or not.
    pub fn with_sensor(mut self, sensor: bool) -> Self {
        self.is_sensor = sensor;
        self
    }

    /// Sets desired solver groups.    
    pub fn with_solver_groups(mut self, solver_groups: InteractionGroups) -> Self {
        self.solver_groups = solver_groups;
        self
    }

    /// Sets desired collision groups.
    pub fn with_collision_groups(mut self, collision_groups: InteractionGroups) -> Self {
        self.collision_groups = collision_groups;
        self
    }

    /// Sets desired friction combine rule.
    pub fn with_friction_combine_rule(mut self, rule: CoefficientCombineRule) -> Self {
        self.friction_combine_rule = rule;
        self
    }

    /// Sets desired restitution combine rule.
    pub fn with_restitution_combine_rule(mut self, rule: CoefficientCombineRule) -> Self {
        self.restitution_combine_rule = rule;
        self
    }

    /// Creates collider node, but does not add it to a graph.
    pub fn build_collider(self) -> Collider {
        Collider {
            base: self.base_builder.build_base(),
            shape: self.shape.into(),
            friction: self.friction.into(),
            density: self.density.into(),
            restitution: self.restitution.into(),
            is_sensor: self.is_sensor.into(),
            collision_groups: self.collision_groups.into(),
            solver_groups: self.solver_groups.into(),
            friction_combine_rule: self.friction_combine_rule.into(),
            restitution_combine_rule: self.restitution_combine_rule.into(),
            native: Cell::new(ColliderHandle::invalid()),
        }
    }

    /// Creates collider node, but does not add it to a graph.
    pub fn build_node(self) -> Node {
        Node::new(self.build_collider())
    }

    /// Creates collider node and adds it to the graph.
    pub fn build(self, graph: &mut Graph) -> Handle<Node> {
        graph.add_node(self.build_node())
    }
}

#[cfg(test)]
mod test {
    use crate::scene::collider::BitMask;
    use crate::scene::{
        base::{test::check_inheritable_properties_equality, BaseBuilder},
        dim2::collider::{Collider, ColliderBuilder, ColliderShape, InteractionGroups},
        graph::physics::CoefficientCombineRule,
        node::NodeTrait,
    };

    #[test]
    fn test_collider_2d_inheritance() {
        let parent = ColliderBuilder::new(BaseBuilder::new())
            .with_shape(ColliderShape::ball(1.0))
            .with_friction(1.0)
            .with_restitution(1.0)
            .with_density(Some(2.0))
            .with_sensor(true)
            .with_restitution_combine_rule(CoefficientCombineRule::Max)
            .with_friction_combine_rule(CoefficientCombineRule::Max)
            .with_collision_groups(InteractionGroups::new(BitMask(1), BitMask(2)))
            .with_solver_groups(InteractionGroups::new(BitMask(1), BitMask(2)))
            .build_node();

        let mut child = ColliderBuilder::new(BaseBuilder::new()).build_collider();

        child.inherit(&parent).unwrap();

        let parent = parent.cast::<Collider>().unwrap();

        check_inheritable_properties_equality(&child.base, &parent.base);
        check_inheritable_properties_equality(&child, parent);
    }
}
