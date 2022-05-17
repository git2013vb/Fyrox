//! Scene physics module.

use crate::{
    core::{
        algebra::{
            DMatrix, Dynamic, Isometry3, Matrix4, Point3, Translation3, UnitQuaternion, VecStorage,
            Vector2, Vector3,
        },
        arrayvec::ArrayVec,
        color::Color,
        inspect::{Inspect, PropertyInfo},
        instant,
        math::{aabb::AxisAlignedBoundingBox, Matrix4Ext},
        pool::Handle,
        visitor::prelude::*,
        BiDirHashMap,
    },
    scene::{
        self,
        collider::{self, ColliderShape, GeometrySource},
        debug::{Line, SceneDrawingContext},
        graph::isometric_global_transform,
        graph::NodePool,
        mesh::{
            buffer::{VertexAttributeUsage, VertexReadTrait},
            Mesh,
        },
        node::Node,
        rigidbody::ApplyAction,
        terrain::Terrain,
        variable::VariableFlags,
    },
    utils::{
        log::{Log, MessageKind},
        raw_mesh::{RawMeshBuilder, RawVertex},
    },
};
use rapier3d::{
    dynamics::{
        CCDSolver, GenericJoint, GenericJointBuilder, ImpulseJointHandle, ImpulseJointSet,
        IslandManager, JointAxesMask, MultibodyJointHandle, MultibodyJointSet, RigidBody,
        RigidBodyActivation, RigidBodyBuilder, RigidBodyHandle, RigidBodySet, RigidBodyType,
        SphericalJointBuilder,
    },
    geometry::{
        BroadPhase, Collider, ColliderBuilder, ColliderHandle, ColliderSet, Cuboid,
        InteractionGroups, NarrowPhase, Ray, Shape, SharedShape, TriMesh,
    },
    math::UnitVector,
    pipeline::{EventHandler, PhysicsPipeline, QueryPipeline},
    prelude::JointAxis,
};
use std::{
    cell::{Cell, RefCell},
    cmp::Ordering,
    fmt::{Debug, Formatter},
    hash::Hash,
    sync::Arc,
    time::Duration,
};
use strum_macros::{AsRefStr, EnumString, EnumVariantNames};

/// Shape-dependent identifier.
#[derive(Copy, Clone, Debug, Hash, PartialEq, Eq)]
pub enum FeatureId {
    /// Shape-dependent identifier of a vertex.
    Vertex(u32),
    /// Shape-dependent identifier of an edge.
    Edge(u32),
    /// Shape-dependent identifier of a face.
    Face(u32),
    /// Unknown identifier.
    Unknown,
}

impl From<rapier3d::geometry::FeatureId> for FeatureId {
    fn from(v: rapier3d::geometry::FeatureId) -> Self {
        match v {
            rapier3d::geometry::FeatureId::Vertex(v) => FeatureId::Vertex(v),
            rapier3d::geometry::FeatureId::Edge(v) => FeatureId::Edge(v),
            rapier3d::geometry::FeatureId::Face(v) => FeatureId::Face(v),
            rapier3d::geometry::FeatureId::Unknown => FeatureId::Unknown,
        }
    }
}

impl From<rapier2d::geometry::FeatureId> for FeatureId {
    fn from(v: rapier2d::geometry::FeatureId) -> Self {
        match v {
            rapier2d::geometry::FeatureId::Vertex(v) => FeatureId::Vertex(v),
            rapier2d::geometry::FeatureId::Face(v) => FeatureId::Face(v),
            rapier2d::geometry::FeatureId::Unknown => FeatureId::Unknown,
        }
    }
}

/// Rules used to combine two coefficients.
///
/// # Notes
///
/// This is used to determine the effective restitution and friction coefficients for a contact
/// between two colliders. Each collider has its combination rule of type `CoefficientCombineRule`,
/// the rule actually used is given by `max(first_combine_rule, second_combine_rule)`.
#[derive(
    Copy, Clone, Debug, PartialEq, Eq, Visit, Inspect, EnumVariantNames, EnumString, AsRefStr,
)]
#[repr(u32)]
pub enum CoefficientCombineRule {
    /// The two coefficients are averaged.
    Average = 0,
    /// The smallest coefficient is chosen.
    Min,
    /// The two coefficients are multiplied.
    Multiply,
    /// The greatest coefficient is chosen.
    Max,
}

impl Default for CoefficientCombineRule {
    fn default() -> Self {
        CoefficientCombineRule::Average
    }
}

impl From<rapier3d::dynamics::CoefficientCombineRule> for CoefficientCombineRule {
    fn from(v: rapier3d::dynamics::CoefficientCombineRule) -> Self {
        match v {
            rapier3d::dynamics::CoefficientCombineRule::Average => CoefficientCombineRule::Average,
            rapier3d::dynamics::CoefficientCombineRule::Min => CoefficientCombineRule::Min,
            rapier3d::dynamics::CoefficientCombineRule::Multiply => {
                CoefficientCombineRule::Multiply
            }
            rapier3d::dynamics::CoefficientCombineRule::Max => CoefficientCombineRule::Max,
        }
    }
}

impl Into<rapier3d::dynamics::CoefficientCombineRule> for CoefficientCombineRule {
    fn into(self) -> rapier3d::dynamics::CoefficientCombineRule {
        match self {
            CoefficientCombineRule::Average => rapier3d::dynamics::CoefficientCombineRule::Average,
            CoefficientCombineRule::Min => rapier3d::dynamics::CoefficientCombineRule::Min,
            CoefficientCombineRule::Multiply => {
                rapier3d::dynamics::CoefficientCombineRule::Multiply
            }
            CoefficientCombineRule::Max => rapier3d::dynamics::CoefficientCombineRule::Max,
        }
    }
}

impl Into<rapier2d::dynamics::CoefficientCombineRule> for CoefficientCombineRule {
    fn into(self) -> rapier2d::dynamics::CoefficientCombineRule {
        match self {
            CoefficientCombineRule::Average => rapier2d::dynamics::CoefficientCombineRule::Average,
            CoefficientCombineRule::Min => rapier2d::dynamics::CoefficientCombineRule::Min,
            CoefficientCombineRule::Multiply => {
                rapier2d::dynamics::CoefficientCombineRule::Multiply
            }
            CoefficientCombineRule::Max => rapier2d::dynamics::CoefficientCombineRule::Max,
        }
    }
}

/// Performance statistics for the physics part of the engine.
#[derive(Debug, Default, Clone)]
pub struct PhysicsPerformanceStatistics {
    /// A time that was needed to perform a single simulation step.
    pub step_time: Duration,

    /// A time that was needed to perform all ray casts.
    pub total_ray_cast_time: Cell<Duration>,
}

impl PhysicsPerformanceStatistics {
    /// Resets performance statistics to default values.
    pub fn reset(&mut self) {
        *self = Default::default();
    }

    /// Returns total amount of time for every part of statistics.
    pub fn total(&self) -> Duration {
        self.step_time + self.total_ray_cast_time.get()
    }
}

/// A ray intersection result.
#[derive(Debug, Clone)]
pub struct Intersection {
    /// A handle of the collider with which intersection was detected.
    pub collider: Handle<Node>,

    /// A normal at the intersection position.
    pub normal: Vector3<f32>,

    /// A position of the intersection in world coordinates.
    pub position: Point3<f32>,

    /// Additional data that contains a kind of the feature with which
    /// intersection was detected as well as its index.
    ///
    /// # Important notes.
    ///
    /// FeatureId::Face might have index that is greater than amount of triangles in
    /// a triangle mesh, this means that intersection was detected from "back" side of
    /// a face. To "fix" that index, simply subtract amount of triangles of a triangle
    /// mesh from the value.
    pub feature: FeatureId,

    /// Distance from the ray origin.
    pub toi: f32,
}

/// A set of options for the ray cast.
pub struct RayCastOptions {
    /// A ray origin.
    pub ray_origin: Point3<f32>,

    /// A ray direction. Can be non-normalized.
    pub ray_direction: Vector3<f32>,

    /// Maximum distance of cast.
    pub max_len: f32,

    /// Groups to check.
    pub groups: collider::InteractionGroups,

    /// Whether to sort intersections from closest to farthest.
    pub sort_results: bool,
}

/// A trait for ray cast results storage. It has two implementations: Vec and ArrayVec.
/// Latter is needed for the cases where you need to avoid runtime memory allocations
/// and do everything on stack.
pub trait QueryResultsStorage {
    /// Pushes new intersection in the storage. Returns true if intersection was
    /// successfully inserted, false otherwise.
    fn push(&mut self, intersection: Intersection) -> bool;

    /// Clears the storage.
    fn clear(&mut self);

    /// Sorts intersections by given compare function.
    fn sort_intersections_by<C: FnMut(&Intersection, &Intersection) -> Ordering>(&mut self, cmp: C);
}

impl QueryResultsStorage for Vec<Intersection> {
    fn push(&mut self, intersection: Intersection) -> bool {
        self.push(intersection);
        true
    }

    fn clear(&mut self) {
        self.clear()
    }

    fn sort_intersections_by<C>(&mut self, cmp: C)
    where
        C: FnMut(&Intersection, &Intersection) -> Ordering,
    {
        self.sort_by(cmp);
    }
}

impl<const CAP: usize> QueryResultsStorage for ArrayVec<Intersection, CAP> {
    fn push(&mut self, intersection: Intersection) -> bool {
        self.try_push(intersection).is_ok()
    }

    fn clear(&mut self) {
        self.clear()
    }

    fn sort_intersections_by<C>(&mut self, cmp: C)
    where
        C: FnMut(&Intersection, &Intersection) -> Ordering,
    {
        self.sort_by(cmp);
    }
}

/// Data of the contact.
pub struct ContactData {
    /// The contact point in the local-space of the first shape.
    pub local_p1: Vector3<f32>,
    /// The contact point in the local-space of the second shape.
    pub local_p2: Vector3<f32>,
    /// The distance between the two contact points.
    pub dist: f32,
    /// The impulse, along the contact normal, applied by this contact to the first collider's rigid-body.
    /// The impulse applied to the second collider's rigid-body is given by `-impulse`.
    pub impulse: f32,
    /// The friction impulses along the basis orthonormal to the contact normal, applied to the first
    /// collider's rigid-body.
    pub tangent_impulse: Vector2<f32>,
}

/// A contact manifold between two colliders.
pub struct ContactManifold {
    /// The contacts points.
    pub points: Vec<ContactData>,
    /// The contact normal of all the contacts of this manifold, expressed in the local space of the first shape.
    pub local_n1: Vector3<f32>,
    /// The contact normal of all the contacts of this manifold, expressed in the local space of the second shape.
    pub local_n2: Vector3<f32>,
    /// The first rigid-body involved in this contact manifold.
    pub rigid_body1: Handle<Node>,
    /// The second rigid-body involved in this contact manifold.
    pub rigid_body2: Handle<Node>,
    /// The world-space contact normal shared by all the contact in this contact manifold.
    pub normal: Vector3<f32>,
}

/// Contact info for pair of colliders.
pub struct ContactPair {
    /// The first collider involved in the contact pair.
    pub collider1: Handle<Node>,
    /// The second collider involved in the contact pair.
    pub collider2: Handle<Node>,
    /// The set of contact manifolds between the two colliders.
    /// All contact manifold contain themselves contact points between the colliders.
    pub manifolds: Vec<ContactManifold>,
    /// Is there any active contact in this contact pair?
    pub has_any_active_contact: bool,
}

pub(super) struct Container<S, A>
where
    A: Hash + Eq + Clone,
{
    set: S,
    map: BiDirHashMap<A, Handle<Node>>,
}

fn convert_joint_params(params: scene::joint::JointParams) -> GenericJoint {
    match params {
        scene::joint::JointParams::BallJoint(v) => SphericalJointBuilder::new()
            .local_anchor1(Point3::from(v.local_anchor1))
            .local_anchor2(Point3::from(v.local_anchor2))
            .limits(JointAxis::AngX, v.limits_angles)
            .limits(JointAxis::AngY, v.limits_angles)
            .limits(JointAxis::AngZ, v.limits_angles)
            .build()
            .into(),
        scene::joint::JointParams::FixedJoint(v) => {
            GenericJointBuilder::new(JointAxesMask::LOCKED_FIXED_AXES)
                .local_frame1(Isometry3 {
                    translation: Translation3 {
                        vector: v.local_anchor1_translation,
                    },
                    rotation: v.local_anchor1_rotation,
                })
                .local_frame2(Isometry3 {
                    translation: Translation3 {
                        vector: v.local_anchor2_translation,
                    },
                    rotation: v.local_anchor2_rotation,
                })
                .build()
        }
        scene::joint::JointParams::PrismaticJoint(v) => {
            GenericJointBuilder::new(JointAxesMask::LOCKED_PRISMATIC_AXES)
                .local_anchor1(Point3::from(v.local_anchor1))
                .local_axis1(UnitVector::new_normalize(v.local_axis1))
                .local_anchor2(Point3::from(v.local_anchor2))
                .local_axis2(UnitVector::new_normalize(v.local_axis2))
                .limits(JointAxis::X, v.limits)
                .build()
        }
        scene::joint::JointParams::RevoluteJoint(v) => {
            GenericJointBuilder::new(JointAxesMask::LOCKED_REVOLUTE_AXES)
                .local_anchor1(Point3::from(v.local_anchor1))
                .local_axis1(UnitVector::new_normalize(v.local_axis1))
                .local_anchor2(Point3::from(v.local_anchor2))
                .local_axis2(UnitVector::new_normalize(v.local_axis2))
                .limits(JointAxis::AngX, v.limits)
                .build()
        }
    }
}

/// Creates new trimesh collider shape from given mesh node. It also bakes scale into
/// vertices of trimesh because rapier does not support collider scaling yet.
fn make_trimesh(
    owner_inv_transform: Matrix4<f32>,
    owner: Handle<Node>,
    sources: &[GeometrySource],
    nodes: &NodePool,
) -> SharedShape {
    let mut mesh_builder = RawMeshBuilder::new(0, 0);

    // Create inverse transform that will discard rotation and translation, but leave scaling and
    // other parameters of global transform.
    // When global transform of node is combined with this transform, we'll get relative transform
    // with scale baked in. We need to do this because root's transform will be synced with body's
    // but we don't want to bake entire transform including root's transform.
    let root_inv_transform = owner_inv_transform;

    for &source in sources {
        if let Some(mesh) = nodes.try_borrow(source.0).and_then(|n| n.cast::<Mesh>()) {
            let global_transform = root_inv_transform * mesh.global_transform();

            for surface in mesh.surfaces() {
                let shared_data = surface.data();
                let shared_data = shared_data.lock();

                let vertices = &shared_data.vertex_buffer;
                for triangle in shared_data.geometry_buffer.iter() {
                    let a = RawVertex::from(
                        global_transform
                            .transform_point(&Point3::from(
                                vertices
                                    .get(triangle[0] as usize)
                                    .unwrap()
                                    .read_3_f32(VertexAttributeUsage::Position)
                                    .unwrap(),
                            ))
                            .coords,
                    );
                    let b = RawVertex::from(
                        global_transform
                            .transform_point(&Point3::from(
                                vertices
                                    .get(triangle[1] as usize)
                                    .unwrap()
                                    .read_3_f32(VertexAttributeUsage::Position)
                                    .unwrap(),
                            ))
                            .coords,
                    );
                    let c = RawVertex::from(
                        global_transform
                            .transform_point(&Point3::from(
                                vertices
                                    .get(triangle[2] as usize)
                                    .unwrap()
                                    .read_3_f32(VertexAttributeUsage::Position)
                                    .unwrap(),
                            ))
                            .coords,
                    );

                    mesh_builder.insert(a);
                    mesh_builder.insert(b);
                    mesh_builder.insert(c);
                }
            }
        }
    }

    let raw_mesh = mesh_builder.build();

    let vertices: Vec<Point3<f32>> = raw_mesh
        .vertices
        .into_iter()
        .map(|v| Point3::new(v.x, v.y, v.z))
        .collect();

    let indices = raw_mesh
        .triangles
        .into_iter()
        .map(|t| [t.0[0], t.0[1], t.0[2]])
        .collect::<Vec<_>>();

    if indices.is_empty() {
        Log::writeln(
            MessageKind::Warning,
            format!(
                "Failed to create triangle mesh collider for {}, it has no vertices!",
                nodes[owner].name()
            ),
        );

        SharedShape::trimesh(vec![Point3::new(0.0, 0.0, 0.0)], vec![[0, 0, 0]])
    } else {
        SharedShape::trimesh(vertices, indices)
    }
}

/// Creates new convex polyhedron collider shape from given mesh node. It also bakes scale into
/// vertices of trimesh because rapier does not support collider scaling yet.
fn make_polyhedron_shape(owner_inv_transform: Matrix4<f32>, mesh: &Mesh) -> SharedShape {
    let mut mesh_builder = RawMeshBuilder::new(0, 0);

    // Create inverse transform that will discard rotation and translation, but leave scaling and
    // other parameters of global transform.
    // When global transform of node is combined with this transform, we'll get relative transform
    // with scale baked in. We need to do this because root's transform will be synced with body's
    // but we don't want to bake entire transform including root's transform.
    let root_inv_transform = owner_inv_transform;

    let global_transform = root_inv_transform * mesh.global_transform();

    for surface in mesh.surfaces() {
        let shared_data = surface.data();
        let shared_data = shared_data.lock();

        let vertices = &shared_data.vertex_buffer;
        for triangle in shared_data.geometry_buffer.iter() {
            let a = RawVertex::from(
                global_transform
                    .transform_point(&Point3::from(
                        vertices
                            .get(triangle[0] as usize)
                            .unwrap()
                            .read_3_f32(VertexAttributeUsage::Position)
                            .unwrap(),
                    ))
                    .coords,
            );
            let b = RawVertex::from(
                global_transform
                    .transform_point(&Point3::from(
                        vertices
                            .get(triangle[1] as usize)
                            .unwrap()
                            .read_3_f32(VertexAttributeUsage::Position)
                            .unwrap(),
                    ))
                    .coords,
            );
            let c = RawVertex::from(
                global_transform
                    .transform_point(&Point3::from(
                        vertices
                            .get(triangle[2] as usize)
                            .unwrap()
                            .read_3_f32(VertexAttributeUsage::Position)
                            .unwrap(),
                    ))
                    .coords,
            );

            mesh_builder.insert(a);
            mesh_builder.insert(b);
            mesh_builder.insert(c);
        }
    }

    let raw_mesh = mesh_builder.build();

    let vertices: Vec<Point3<f32>> = raw_mesh
        .vertices
        .into_iter()
        .map(|v| Point3::new(v.x, v.y, v.z))
        .collect();

    let indices = raw_mesh
        .triangles
        .into_iter()
        .map(|t| [t.0[0], t.0[1], t.0[2]])
        .collect::<Vec<_>>();

    SharedShape::convex_decomposition(&vertices, &indices)
}

/// Creates height field shape from given terrain.
fn make_heightfield(terrain: &Terrain) -> SharedShape {
    assert!(!terrain.chunks_ref().is_empty());

    // Count rows and columns.
    let first_chunk = terrain.chunks_ref().first().unwrap();
    let chunk_size = Vector2::new(
        first_chunk.width_point_count(),
        first_chunk.length_point_count(),
    );
    let nrows = chunk_size.y * terrain.length_chunk_count() as u32;
    let ncols = chunk_size.x * terrain.width_chunk_count() as u32;

    // Combine height map of each chunk into bigger one.
    let mut ox = 0;
    let mut oz = 0;
    let mut data = vec![0.0; (nrows * ncols) as usize];
    for cz in 0..terrain.length_chunk_count() {
        for cx in 0..terrain.width_chunk_count() {
            let chunk = &terrain.chunks_ref()[cz * terrain.width_chunk_count() + cx];

            for z in 0..chunk.length_point_count() {
                for x in 0..chunk.width_point_count() {
                    let value = chunk.heightmap()[(z * chunk.width_point_count() + x) as usize];
                    data[((ox + x) * nrows + oz + z) as usize] = value;
                }
            }

            ox += chunk_size.x;
        }

        ox = 0;
        oz += chunk_size.y;
    }

    SharedShape::heightfield(
        DMatrix::from_data(VecStorage::new(
            Dynamic::new(nrows as usize),
            Dynamic::new(ncols as usize),
            data,
        )),
        Vector3::new(terrain.width(), 1.0, terrain.length()),
    )
}

// Converts descriptor in a shared shape.
fn collider_shape_into_native_shape(
    shape: &ColliderShape,
    owner_inv_global_transform: Matrix4<f32>,
    owner_collider: Handle<Node>,
    pool: &NodePool,
) -> Option<SharedShape> {
    match shape {
        ColliderShape::Ball(ball) => Some(SharedShape::ball(ball.radius)),

        ColliderShape::Cylinder(cylinder) => {
            Some(SharedShape::cylinder(cylinder.half_height, cylinder.radius))
        }
        ColliderShape::Cone(cone) => Some(SharedShape::cone(cone.half_height, cone.radius)),
        ColliderShape::Cuboid(cuboid) => {
            Some(SharedShape(Arc::new(Cuboid::new(cuboid.half_extents))))
        }
        ColliderShape::Capsule(capsule) => Some(SharedShape::capsule(
            Point3::from(capsule.begin),
            Point3::from(capsule.end),
            capsule.radius,
        )),
        ColliderShape::Segment(segment) => Some(SharedShape::segment(
            Point3::from(segment.begin),
            Point3::from(segment.end),
        )),
        ColliderShape::Triangle(triangle) => Some(SharedShape::triangle(
            Point3::from(triangle.a),
            Point3::from(triangle.b),
            Point3::from(triangle.c),
        )),
        ColliderShape::Trimesh(trimesh) => {
            if trimesh.sources.is_empty() {
                None
            } else {
                Some(make_trimesh(
                    owner_inv_global_transform,
                    owner_collider,
                    &trimesh.sources,
                    pool,
                ))
            }
        }
        ColliderShape::Heightfield(heightfield) => pool
            .try_borrow(heightfield.geometry_source.0)
            .and_then(|n| n.cast::<Terrain>())
            .map(make_heightfield),
        ColliderShape::Polyhedron(polyhedron) => pool
            .try_borrow(polyhedron.geometry_source.0)
            .and_then(|n| n.cast::<Mesh>())
            .map(|mesh| make_polyhedron_shape(owner_inv_global_transform, mesh)),
    }
}

/// Parameters for a time-step of the physics engine.
///
/// # Notes
///
/// This is almost one-to-one copy of Rapier's integration parameters with custom attributes for
/// each parameter.
#[derive(Copy, Clone, Visit, Inspect, Debug)]
pub struct IntegrationParameters {
    /// The timestep length (default: `1.0 / 60.0`)
    #[inspect(
        min_value = 0.0,
        description = "The timestep length (default: `1.0 / 60.0`)"
    )]
    pub dt: f32,

    /// Minimum timestep size when using CCD with multiple substeps (default `1.0 / 60.0 / 100.0`)
    ///
    /// When CCD with multiple substeps is enabled, the timestep is subdivided into smaller pieces.
    /// This timestep subdivision won't generate timestep lengths smaller than `min_ccd_dt`.
    ///
    /// Setting this to a large value will reduce the opportunity to performing CCD substepping,
    /// resulting in potentially more time dropped by the motion-clamping mechanism. Setting this
    /// to an very small value may lead to numerical instabilities.
    #[inspect(
        min_value = 0.0,
        description = "Minimum timestep size when using CCD with multiple\
         substeps (default `1.0 / 60.0 / 100.0`)"
    )]
    pub min_ccd_dt: f32,

    /// The Error Reduction Parameter in `[0, 1]` is the proportion of the positional error to be
    /// corrected at each time step (default: `0.2`).
    #[inspect(
        min_value = 0.0,
        max_value = 1.0,
        description = "The Error Reduction Parameter in `[0, 1]` is the proportion of the \
        positional error to be corrected at each time step (default: `0.2`)"
    )]
    pub erp: f32,

    /// 0-1: the damping ratio used by the springs.
    /// Lower values make the constraints more compliant (more "springy", allowing more visible penetrations
    /// before stabilization).
    /// (default `0.25`).
    #[inspect(
        min_value = 0.0,
        max_value = 1.0,
        description = "The damping ratio used by the springs in `[0, 1]` Lower values make the constraints more \
     compliant (more springy, allowing more visible penetrations before stabilization). Default `0.25`"
    )]
    pub damping_ratio: f32,

    /// The Error Reduction Parameter for joints in `[0, 1]` is the proportion of the positional
    /// error to be corrected at each time step (default: `0.2`).
    #[inspect(
        min_value = 0.0,
        max_value = 1.0,
        description = "The Error Reduction Parameter for joints in `[0, 1]` is the proportion \
        of the positional error to be corrected at each time step (default: `0.2`)."
    )]
    pub joint_erp: f32,

    /// The fraction of critical damping applied to the joint for constraints regularization.
    /// (default `0.25`).
    #[inspect(
        min_value = 0.0,
        description = "The fraction of critical damping applied to the joint for \
        constraints regularization (default: `0.25`)."
    )]
    pub joint_damping_ratio: f32,

    /// Amount of penetration the engine wont attempt to correct (default: `0.005m`).
    #[inspect(
        min_value = 0.0,
        description = "Amount of penetration the engine wont attempt to correct (default: `0.005m`)."
    )]
    pub allowed_linear_error: f32,

    /// Maximum amount of penetration the solver will attempt to resolve in one timestep.
    #[inspect(
        min_value = 0.0,
        description = "Maximum amount of penetration the solver will attempt to resolve in one timestep."
    )]
    pub max_penetration_correction: f32,

    /// The maximal distance separating two objects that will generate predictive contacts (default: `0.002`).
    #[inspect(
        min_value = 0.0,
        description = "The maximal distance separating two objects that will generate \
        predictive contacts (default: `0.002`)."
    )]
    pub prediction_distance: f32,

    /// Maximum number of iterations performed by the velocity constraints solver (default: `4`).
    #[inspect(
        min_value = 0.0,
        description = "Maximum number of iterations performed by the \
    velocity constraints solver (default: `4`)."
    )]
    pub max_velocity_iterations: u32,

    /// Maximum number of iterations performed to solve friction constraints (default: `8`).
    #[inspect(
        min_value = 0.0,
        description = "Maximum number of iterations performed to solve friction constraints (default: `8`)"
    )]
    pub max_velocity_friction_iterations: u32,

    /// Maximum number of iterations performed to remove the energy introduced by penetration corrections  (default: `1`).
    #[inspect(
        min_value = 0.0,
        description = "Maximum number of iterations performed to remove the energy introduced by penetration corrections  (default: `1`)."
    )]
    pub max_stabilization_iterations: u32,

    /// If `false`, friction and non-penetration constraints will be solved in the same loop. Otherwise,
    /// non-penetration constraints are solved first, and friction constraints are solved after (default: `true`).
    #[inspect(
        description = "If `false`, friction and non-penetration constraints will be solved in the same loop. Otherwise, \
        non-penetration constraints are solved first, and friction constraints are solved after (default: `true`)."
    )]
    pub interleave_restitution_and_friction_resolution: bool,

    /// Minimum number of dynamic bodies in each active island (default: `128`).
    #[inspect(
        min_value = 0.0,
        description = "Minimum number of dynamic bodies in each active island (default: `128`)."
    )]
    pub min_island_size: u32,

    /// Maximum number of substeps performed by the  solver (default: `1`).
    #[inspect(
        min_value = 0.0,
        description = "Maximum number of substeps performed by the  solver (default: `1`)."
    )]
    pub max_ccd_substeps: u32,
}

impl Default for IntegrationParameters {
    fn default() -> Self {
        Self {
            dt: 1.0 / 60.0,
            min_ccd_dt: 1.0 / 60.0 / 100.0,
            erp: 0.8,
            damping_ratio: 0.25,
            joint_erp: 1.0,
            joint_damping_ratio: 1.0,
            allowed_linear_error: 0.001,
            max_penetration_correction: f32::MAX,
            prediction_distance: 0.002,
            max_velocity_iterations: 4,
            max_velocity_friction_iterations: 8,
            max_stabilization_iterations: 1,
            interleave_restitution_and_friction_resolution: true,
            min_island_size: 128,
            max_ccd_substeps: 1,
        }
    }
}

/// Physics world is responsible for physics simulation in the engine. There is a very few public
/// methods, mostly for ray casting. You should add physical entities using scene graph nodes, such
/// as RigidBody, Collider, Joint.
#[derive(Visit, Inspect)]
pub struct PhysicsWorld {
    /// A flag that defines whether physics simulation is enabled or not.
    pub enabled: bool,

    /// A set of parameters that define behavior of every rigid body.
    pub integration_parameters: IntegrationParameters,

    /// Current gravity vector. Default is (0.0, -9.81, 0.0)
    pub gravity: Vector3<f32>,

    /// Performance statistics of a single simulation step.
    #[visit(skip)]
    #[inspect(skip)]
    pub performance_statistics: PhysicsPerformanceStatistics,

    // Current physics pipeline.
    #[visit(skip)]
    #[inspect(skip)]
    pipeline: PhysicsPipeline,
    // Broad phase performs rough intersection checks.
    #[visit(skip)]
    #[inspect(skip)]
    broad_phase: BroadPhase,
    // Narrow phase is responsible for precise contact generation.
    #[visit(skip)]
    #[inspect(skip)]
    narrow_phase: NarrowPhase,
    // A continuous collision detection solver.
    #[visit(skip)]
    #[inspect(skip)]
    ccd_solver: CCDSolver,
    // Structure responsible for maintaining the set of active rigid-bodies, and putting non-moving
    // rigid-bodies to sleep to save computation times.
    #[visit(skip)]
    #[inspect(skip)]
    islands: IslandManager,
    // A container of rigid bodies.
    #[visit(skip)]
    #[inspect(skip)]
    bodies: Container<RigidBodySet, RigidBodyHandle>,
    // A container of colliders.
    #[visit(skip)]
    #[inspect(skip)]
    colliders: Container<ColliderSet, ColliderHandle>,
    // A container of impulse joints.
    #[visit(skip)]
    #[inspect(skip)]
    joints: Container<ImpulseJointSet, ImpulseJointHandle>,
    // A container of multibody joints.
    #[visit(skip)]
    #[inspect(skip)]
    multibody_joints: Container<MultibodyJointSet, MultibodyJointHandle>,
    // Event handler collects info about contacts and proximity events.
    #[visit(skip)]
    #[inspect(skip)]
    event_handler: Box<dyn EventHandler>,
    #[visit(skip)]
    #[inspect(skip)]
    query: RefCell<QueryPipeline>,
}

fn draw_shape(shape: &dyn Shape, transform: Matrix4<f32>, context: &mut SceneDrawingContext) {
    if let Some(trimesh) = shape.as_trimesh() {
        let trimesh: &TriMesh = trimesh;
        for triangle in trimesh.triangles() {
            let a = transform.transform_point(&triangle.a);
            let b = transform.transform_point(&triangle.b);
            let c = transform.transform_point(&triangle.c);
            context.draw_triangle(a.coords, b.coords, c.coords, Color::opaque(200, 200, 200));
        }
    } else if let Some(cuboid) = shape.as_cuboid() {
        let min = -cuboid.half_extents;
        let max = cuboid.half_extents;
        context.draw_oob(
            &AxisAlignedBoundingBox::from_min_max(min, max),
            transform,
            Color::opaque(200, 200, 200),
        );
    } else if let Some(ball) = shape.as_ball() {
        context.draw_sphere(
            Vector3::new(transform[12], transform[13], transform[14]),
            10,
            10,
            ball.radius,
            Color::opaque(200, 200, 200),
        );
    } else if let Some(cone) = shape.as_cone() {
        context.draw_cone(
            10,
            cone.radius,
            cone.half_height * 2.0,
            transform,
            Color::opaque(200, 200, 200),
        );
    } else if let Some(cylinder) = shape.as_cylinder() {
        context.draw_cylinder(
            10,
            cylinder.radius,
            cylinder.half_height * 2.0,
            true,
            transform,
            Color::opaque(200, 200, 200),
        );
    } else if let Some(round_cylinder) = shape.as_round_cylinder() {
        context.draw_cylinder(
            10,
            round_cylinder.inner_shape.radius,
            round_cylinder.inner_shape.half_height * 2.0,
            false,
            transform,
            Color::opaque(200, 200, 200),
        );
    } else if let Some(triangle) = shape.as_triangle() {
        context.draw_triangle(
            triangle.a.coords,
            triangle.b.coords,
            triangle.c.coords,
            Color::opaque(200, 200, 200),
        );
    } else if let Some(capsule) = shape.as_capsule() {
        context.draw_segment_capsule(
            capsule.segment.a.coords,
            capsule.segment.b.coords,
            capsule.radius,
            10,
            10,
            transform,
            Color::opaque(200, 200, 200),
        );
    } else if let Some(heightfield) = shape.as_heightfield() {
        for triangle in heightfield.triangles() {
            let a = transform.transform_point(&triangle.a);
            let b = transform.transform_point(&triangle.b);
            let c = transform.transform_point(&triangle.c);
            context.draw_triangle(a.coords, b.coords, c.coords, Color::opaque(200, 200, 200));
        }
    } else if let Some(polyhedron) = shape.as_convex_polyhedron() {
        for edge in polyhedron.edges() {
            let ia = edge.vertices.x as usize;
            let ib = edge.vertices.y as usize;

            let pa = transform.transform_point(&polyhedron.points()[ia]).coords;
            let pb = transform.transform_point(&polyhedron.points()[ib]).coords;

            context.add_line(Line {
                begin: pa,
                end: pb,
                color: Color::opaque(200, 200, 200),
            })
        }
    } else if let Some(compound) = shape.as_compound() {
        for (isometry, shape) in compound.shapes() {
            let child_transform = isometry.to_homogeneous() * transform;
            draw_shape(&**shape, child_transform, context);
        }
    }
}

fn isometry_from_global_transform(transform: &Matrix4<f32>) -> Isometry3<f32> {
    Isometry3 {
        translation: Translation3::new(transform[12], transform[13], transform[14]),
        rotation: UnitQuaternion::from_matrix(&transform.basis()),
    }
}

impl PhysicsWorld {
    /// Creates a new instance of the physics world.
    pub(super) fn new() -> Self {
        Self {
            enabled: true,
            pipeline: PhysicsPipeline::new(),
            gravity: Vector3::new(0.0, -9.81, 0.0),
            integration_parameters: IntegrationParameters::default(),
            broad_phase: BroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            ccd_solver: CCDSolver::new(),
            islands: IslandManager::new(),
            bodies: Container {
                set: RigidBodySet::new(),
                map: Default::default(),
            },
            colliders: Container {
                set: ColliderSet::new(),
                map: Default::default(),
            },
            joints: Container {
                set: ImpulseJointSet::new(),
                map: Default::default(),
            },
            multibody_joints: Container {
                set: MultibodyJointSet::new(),
                map: Default::default(),
            },
            event_handler: Box::new(()),
            query: RefCell::new(Default::default()),
            performance_statistics: Default::default(),
        }
    }

    pub(super) fn update(&mut self) {
        let time = instant::Instant::now();

        if self.enabled {
            let integration_parameters = rapier3d::dynamics::IntegrationParameters {
                dt: self.integration_parameters.dt,
                min_ccd_dt: self.integration_parameters.min_ccd_dt,
                erp: self.integration_parameters.erp,
                damping_ratio: self.integration_parameters.damping_ratio,
                joint_erp: self.integration_parameters.joint_erp,
                joint_damping_ratio: self.integration_parameters.joint_damping_ratio,
                allowed_linear_error: self.integration_parameters.allowed_linear_error,
                max_penetration_correction: self.integration_parameters.max_penetration_correction,
                prediction_distance: self.integration_parameters.prediction_distance,
                max_velocity_iterations: self.integration_parameters.max_velocity_iterations
                    as usize,
                max_velocity_friction_iterations: self
                    .integration_parameters
                    .max_velocity_friction_iterations
                    as usize,
                max_stabilization_iterations: self
                    .integration_parameters
                    .max_stabilization_iterations
                    as usize,
                interleave_restitution_and_friction_resolution: self
                    .integration_parameters
                    .interleave_restitution_and_friction_resolution,
                min_island_size: self.integration_parameters.min_island_size as usize,
                max_ccd_substeps: self.integration_parameters.max_ccd_substeps as usize,
            };

            self.pipeline.step(
                &self.gravity,
                &integration_parameters,
                &mut self.islands,
                &mut self.broad_phase,
                &mut self.narrow_phase,
                &mut self.bodies.set,
                &mut self.colliders.set,
                &mut self.joints.set,
                &mut self.multibody_joints.set,
                &mut self.ccd_solver,
                &(),
                &*self.event_handler,
            );
        }

        self.performance_statistics.step_time += instant::Instant::now() - time;
    }

    pub(super) fn add_body(&mut self, owner: Handle<Node>, body: RigidBody) -> RigidBodyHandle {
        let handle = self.bodies.set.insert(body);
        self.bodies.map.insert(handle, owner);
        handle
    }

    pub(crate) fn remove_body(&mut self, handle: RigidBodyHandle) {
        assert!(self.bodies.map.remove_by_key(&handle).is_some());
        self.bodies.set.remove(
            handle,
            &mut self.islands,
            &mut self.colliders.set,
            &mut self.joints.set,
            &mut self.multibody_joints.set,
            true,
        );
    }

    pub(super) fn add_collider(
        &mut self,
        owner: Handle<Node>,
        parent_body: RigidBodyHandle,
        collider: Collider,
    ) -> ColliderHandle {
        let handle =
            self.colliders
                .set
                .insert_with_parent(collider, parent_body, &mut self.bodies.set);
        self.colliders.map.insert(handle, owner);
        handle
    }

    pub(crate) fn remove_collider(&mut self, handle: ColliderHandle) -> bool {
        if self
            .colliders
            .set
            .remove(handle, &mut self.islands, &mut self.bodies.set, false)
            .is_some()
        {
            assert!(self.colliders.map.remove_by_key(&handle).is_some());
            true
        } else {
            false
        }
    }

    pub(super) fn add_joint(
        &mut self,
        owner: Handle<Node>,
        body1: RigidBodyHandle,
        body2: RigidBodyHandle,
        joint: GenericJoint,
    ) -> ImpulseJointHandle {
        let handle = self.joints.set.insert(body1, body2, joint);
        self.joints.map.insert(handle, owner);
        handle
    }

    pub(crate) fn remove_joint(&mut self, handle: ImpulseJointHandle) {
        assert!(self.joints.map.remove_by_key(&handle).is_some());
        self.joints
            .set
            .remove(handle, &mut self.islands, &mut self.bodies.set, false);
    }

    /// Draws physics world. Very useful for debugging, it allows you to see where are
    /// rigid bodies, which colliders they have and so on.
    pub fn draw(&self, context: &mut SceneDrawingContext) {
        for (_, body) in self.bodies.set.iter() {
            context.draw_transform(body.position().to_homogeneous());
        }

        for (_, collider) in self.colliders.set.iter() {
            let body = self.bodies.set.get(collider.parent().unwrap()).unwrap();
            let collider_local_transform = collider.position_wrt_parent().unwrap().to_homogeneous();
            let transform = body.position().to_homogeneous() * collider_local_transform;
            draw_shape(collider.shape(), transform, context);
        }
    }

    /// Casts a ray with given options.
    pub fn cast_ray<S: QueryResultsStorage>(&self, opts: RayCastOptions, query_buffer: &mut S) {
        let time = instant::Instant::now();

        let mut query = self.query.borrow_mut();

        // TODO: Ideally this must be called once per frame, but it seems to be impossible because
        // a body can be deleted during the consecutive calls of this method which will most
        // likely end up in panic because of invalid handle stored in internal acceleration
        // structure. This could be fixed by delaying deleting of bodies/collider to the end
        // of the frame.
        query.update(&self.islands, &self.bodies.set, &self.colliders.set);

        query_buffer.clear();
        let ray = Ray::new(
            opts.ray_origin,
            opts.ray_direction
                .try_normalize(f32::EPSILON)
                .unwrap_or_default(),
        );
        query.intersections_with_ray(
            &self.colliders.set,
            &ray,
            opts.max_len,
            true,
            InteractionGroups::new(opts.groups.memberships, opts.groups.filter),
            None, // TODO
            |handle, intersection| {
                query_buffer.push(Intersection {
                    collider: self.colliders.map.value_of(&handle).cloned().unwrap(),
                    normal: intersection.normal,
                    position: ray.point_at(intersection.toi),
                    feature: intersection.feature.into(),
                    toi: intersection.toi,
                })
            },
        );
        if opts.sort_results {
            query_buffer.sort_intersections_by(|a, b| {
                if a.toi > b.toi {
                    Ordering::Greater
                } else if a.toi < b.toi {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            })
        }

        self.performance_statistics.total_ray_cast_time.set(
            self.performance_statistics.total_ray_cast_time.get()
                + (instant::Instant::now() - time),
        );
    }

    pub(crate) fn set_rigid_body_position(
        &mut self,
        rigid_body: &scene::rigidbody::RigidBody,
        new_global_transform: &Matrix4<f32>,
    ) {
        if let Some(native) = self.bodies.set.get_mut(rigid_body.native.get()) {
            native.set_position(
                isometry_from_global_transform(new_global_transform),
                // Do not wake up body, it is too expensive and must be done **only** by explicit
                // `wake_up` call!
                false,
            );
        }
    }

    pub(crate) fn sync_rigid_body_node(
        &mut self,
        rigid_body: &mut scene::rigidbody::RigidBody,
        parent_transform: Matrix4<f32>,
    ) {
        if self.enabled {
            if let Some(native) = self.bodies.set.get(rigid_body.native.get()) {
                if native.body_type() == RigidBodyType::Dynamic {
                    let local_transform: Matrix4<f32> = parent_transform
                        .try_inverse()
                        .unwrap_or_else(Matrix4::identity)
                        * native.position().to_homogeneous();

                    let local_rotation = UnitQuaternion::from_matrix(&local_transform.basis());
                    let local_position = Vector3::new(
                        local_transform[12],
                        local_transform[13],
                        local_transform[14],
                    );

                    rigid_body
                        .local_transform
                        .set_position(local_position)
                        .set_rotation(local_rotation);
                    rigid_body
                        .lin_vel
                        .set_with_flags(*native.linvel(), VariableFlags::MODIFIED);
                    rigid_body
                        .ang_vel
                        .set_with_flags(*native.angvel(), VariableFlags::MODIFIED);
                    rigid_body.sleeping = native.is_sleeping();
                }
            }
        }
    }

    pub(crate) fn sync_to_rigid_body_node(
        &mut self,
        handle: Handle<Node>,
        rigid_body_node: &scene::rigidbody::RigidBody,
    ) {
        // Important notes!
        // 1) `get_mut` is **very** expensive because it forces physics engine to recalculate contacts
        //    and a lot of other stuff, this is why we need `anything_changed` flag.
        if rigid_body_node.native.get() != RigidBodyHandle::invalid() {
            let mut actions = rigid_body_node.actions.lock();
            if rigid_body_node.need_sync_model() || !actions.is_empty() {
                if let Some(native) = self.bodies.set.get_mut(rigid_body_node.native.get()) {
                    // Sync native rigid body's properties with scene node's in case if they
                    // were changed by user.
                    rigid_body_node
                        .body_type
                        .try_sync_model(|v| native.set_body_type(v.into()));
                    rigid_body_node
                        .lin_vel
                        .try_sync_model(|v| native.set_linvel(v, false));
                    rigid_body_node
                        .ang_vel
                        .try_sync_model(|v| native.set_angvel(v, false));
                    rigid_body_node.mass.try_sync_model(|v| {
                        let mut props = *native.mass_properties();
                        props.set_mass(v, true);
                        native.set_additional_mass_properties(props, true)
                    });
                    rigid_body_node
                        .lin_damping
                        .try_sync_model(|v| native.set_linear_damping(v));
                    rigid_body_node
                        .ang_damping
                        .try_sync_model(|v| native.set_angular_damping(v));
                    rigid_body_node
                        .ccd_enabled
                        .try_sync_model(|v| native.enable_ccd(v));
                    rigid_body_node.can_sleep.try_sync_model(|v| {
                        let mut activation = native.activation_mut();
                        if v {
                            activation.linear_threshold =
                                RigidBodyActivation::default_linear_threshold();
                            activation.angular_threshold =
                                RigidBodyActivation::default_angular_threshold();
                        } else {
                            activation.sleeping = false;
                            activation.linear_threshold = -1.0;
                            activation.angular_threshold = -1.0;
                        };
                    });
                    rigid_body_node
                        .translation_locked
                        .try_sync_model(|v| native.lock_translations(v, false));
                    rigid_body_node.x_rotation_locked.try_sync_model(|v| {
                        native.restrict_rotations(
                            !v,
                            !native.is_rotation_locked()[1],
                            !native.is_rotation_locked()[2],
                            false,
                        );
                    });
                    rigid_body_node.y_rotation_locked.try_sync_model(|v| {
                        native.restrict_rotations(
                            !native.is_rotation_locked()[0],
                            !v,
                            !native.is_rotation_locked()[2],
                            false,
                        );
                    });
                    rigid_body_node.z_rotation_locked.try_sync_model(|v| {
                        native.restrict_rotations(
                            !native.is_rotation_locked()[0],
                            !native.is_rotation_locked()[1],
                            !v,
                            false,
                        );
                    });
                    rigid_body_node
                        .dominance
                        .try_sync_model(|v| native.set_dominance_group(v));
                    rigid_body_node
                        .gravity_scale
                        .try_sync_model(|v| native.set_gravity_scale(v, false));

                    while let Some(action) = actions.pop_front() {
                        match action {
                            ApplyAction::Force(force) => native.add_force(force, false),
                            ApplyAction::Torque(torque) => native.add_torque(torque, false),
                            ApplyAction::ForceAtPoint { force, point } => {
                                native.add_force_at_point(force, Point3::from(point), false)
                            }
                            ApplyAction::Impulse(impulse) => native.apply_impulse(impulse, false),
                            ApplyAction::TorqueImpulse(impulse) => {
                                native.apply_torque_impulse(impulse, false)
                            }
                            ApplyAction::ImpulseAtPoint { impulse, point } => {
                                native.apply_impulse_at_point(impulse, Point3::from(point), false)
                            }
                            ApplyAction::WakeUp => native.wake_up(false),
                        }
                    }
                }
            }
        } else {
            let mut builder = RigidBodyBuilder::new(rigid_body_node.body_type().into())
                .position(isometry_from_global_transform(
                    &rigid_body_node.global_transform(),
                ))
                .ccd_enabled(rigid_body_node.is_ccd_enabled())
                .additional_mass(rigid_body_node.mass())
                .angvel(*rigid_body_node.ang_vel)
                .linvel(*rigid_body_node.lin_vel)
                .linear_damping(*rigid_body_node.lin_damping)
                .angular_damping(*rigid_body_node.ang_damping)
                .can_sleep(rigid_body_node.is_can_sleep())
                .sleeping(rigid_body_node.is_sleeping())
                .dominance_group(rigid_body_node.dominance())
                .gravity_scale(rigid_body_node.gravity_scale())
                .restrict_rotations(
                    !rigid_body_node.is_x_rotation_locked(),
                    !rigid_body_node.is_y_rotation_locked(),
                    !rigid_body_node.is_z_rotation_locked(),
                );

            if rigid_body_node.is_translation_locked() {
                builder = builder.lock_translations();
            }

            rigid_body_node
                .native
                .set(self.add_body(handle, builder.build()));

            Log::writeln(
                MessageKind::Information,
                format!(
                    "Native rigid body was created for node {}",
                    rigid_body_node.name()
                ),
            );
        }
    }

    pub(crate) fn sync_to_collider_node(
        &mut self,
        nodes: &NodePool,
        handle: Handle<Node>,
        collider_node: &scene::collider::Collider,
    ) {
        let anything_changed =
            collider_node.transform_modified.get() || collider_node.needs_sync_model();

        // Important notes!
        // 1) The collider node may lack backing native physics collider in case if it
        //    is not attached to a rigid body.
        // 2) `get_mut` is **very** expensive because it forces physics engine to recalculate contacts
        //    and a lot of other stuff, this is why we need `anything_changed` flag.
        if collider_node.native.get() != ColliderHandle::invalid() {
            if anything_changed {
                if let Some(native) = self.colliders.set.get_mut(collider_node.native.get()) {
                    if collider_node.transform_modified.get() {
                        native.set_position_wrt_parent(Isometry3 {
                            rotation: **collider_node.local_transform().rotation(),
                            translation: Translation3 {
                                vector: **collider_node.local_transform().position(),
                            },
                        });
                    }

                    collider_node.shape.try_sync_model(|v| {
                        let inv_global_transform = isometric_global_transform(nodes, handle)
                            .try_inverse()
                            .unwrap();
                        if let Some(shape) = collider_shape_into_native_shape(
                            &v,
                            inv_global_transform,
                            handle,
                            nodes,
                        ) {
                            native.set_shape(shape);
                        }
                    });
                    collider_node
                        .restitution
                        .try_sync_model(|v| native.set_restitution(v));
                    collider_node.collision_groups.try_sync_model(|v| {
                        native.set_collision_groups(InteractionGroups::new(v.memberships, v.filter))
                    });
                    collider_node.solver_groups.try_sync_model(|v| {
                        native.set_solver_groups(InteractionGroups::new(v.memberships, v.filter))
                    });
                    collider_node
                        .friction
                        .try_sync_model(|v| native.set_friction(v));
                    collider_node
                        .is_sensor
                        .try_sync_model(|v| native.set_sensor(v));
                    collider_node
                        .friction_combine_rule
                        .try_sync_model(|v| native.set_friction_combine_rule(v.into()));
                    collider_node
                        .restitution_combine_rule
                        .try_sync_model(|v| native.set_restitution_combine_rule(v.into()));
                }
            }
        } else if let Some(parent_body) = nodes
            .try_borrow(collider_node.parent())
            .and_then(|n| n.cast::<scene::rigidbody::RigidBody>())
        {
            if parent_body.native.get() != RigidBodyHandle::invalid() {
                let inv_global_transform = isometric_global_transform(nodes, handle)
                    .try_inverse()
                    .unwrap();
                let rigid_body_native = parent_body.native.get();
                if let Some(shape) = collider_shape_into_native_shape(
                    collider_node.shape(),
                    inv_global_transform,
                    handle,
                    nodes,
                ) {
                    let mut builder = ColliderBuilder::new(shape)
                        .position(Isometry3 {
                            rotation: **collider_node.local_transform().rotation(),
                            translation: Translation3 {
                                vector: **collider_node.local_transform().position(),
                            },
                        })
                        .friction(collider_node.friction())
                        .restitution(collider_node.restitution())
                        .collision_groups(InteractionGroups::new(
                            collider_node.collision_groups().memberships,
                            collider_node.collision_groups().filter,
                        ))
                        .friction_combine_rule(collider_node.friction_combine_rule().into())
                        .restitution_combine_rule(collider_node.restitution_combine_rule().into())
                        .solver_groups(InteractionGroups::new(
                            collider_node.solver_groups().memberships,
                            collider_node.solver_groups().filter,
                        ))
                        .sensor(collider_node.is_sensor());

                    if let Some(density) = collider_node.density() {
                        builder = builder.density(density);
                    }

                    let native_handle =
                        self.add_collider(handle, rigid_body_native, builder.build());

                    collider_node.native.set(native_handle);

                    Log::writeln(
                        MessageKind::Information,
                        format!(
                            "Native collider was created for node {}",
                            collider_node.name()
                        ),
                    );
                }
            }
        }
    }

    pub(crate) fn sync_to_joint_node(
        &mut self,
        nodes: &NodePool,
        handle: Handle<Node>,
        joint: &scene::joint::Joint,
    ) {
        if let Some(native) = self.joints.set.get_mut(joint.native.get()) {
            joint
                .params
                .try_sync_model(|v| native.data = convert_joint_params(v));
            joint.body1.try_sync_model(|v| {
                if let Some(rigid_body_node) = nodes
                    .try_borrow(v)
                    .and_then(|n| n.cast::<scene::rigidbody::RigidBody>())
                {
                    native.body1 = rigid_body_node.native.get();
                }
            });
            joint.body2.try_sync_model(|v| {
                if let Some(rigid_body_node) = nodes
                    .try_borrow(v)
                    .and_then(|n| n.cast::<scene::rigidbody::RigidBody>())
                {
                    native.body2 = rigid_body_node.native.get();
                }
            });
        } else {
            let body1_handle = joint.body1();
            let body2_handle = joint.body2();
            let params = joint.params().clone();

            // A native joint can be created iff both rigid bodies are correctly assigned.
            if let (Some(body1), Some(body2)) = (
                nodes
                    .try_borrow(body1_handle)
                    .and_then(|n| n.cast::<scene::rigidbody::RigidBody>()),
                nodes
                    .try_borrow(body2_handle)
                    .and_then(|n| n.cast::<scene::rigidbody::RigidBody>()),
            ) {
                let native_body1 = body1.native.get();
                let native_body2 = body2.native.get();

                let native = self.add_joint(
                    handle,
                    native_body1,
                    native_body2,
                    convert_joint_params(params),
                );

                joint.native.set(native);

                Log::writeln(
                    MessageKind::Information,
                    format!("Native joint was created for node {}", joint.name()),
                );
            }
        }
    }

    pub(crate) fn contacts_with(
        &self,
        collider: ColliderHandle,
    ) -> impl Iterator<Item = ContactPair> + '_ {
        self.narrow_phase
            .contacts_with(collider)
            .map(|c| ContactPair {
                collider1: self
                    .colliders
                    .map
                    .value_of(&c.collider1)
                    .cloned()
                    .unwrap_or_default(),
                collider2: self
                    .colliders
                    .map
                    .value_of(&c.collider2)
                    .cloned()
                    .unwrap_or_default(),
                manifolds: c
                    .manifolds
                    .iter()
                    .map(|m| ContactManifold {
                        points: m
                            .points
                            .iter()
                            .map(|p| ContactData {
                                local_p1: p.local_p1.coords,
                                local_p2: p.local_p2.coords,
                                dist: p.dist,
                                impulse: p.data.impulse,
                                tangent_impulse: p.data.tangent_impulse,
                            })
                            .collect(),
                        local_n1: m.local_n1,
                        local_n2: m.local_n2,
                        rigid_body1: m
                            .data
                            .rigid_body1
                            .and_then(|h| self.bodies.map.value_of(&h).cloned())
                            .unwrap_or_default(),
                        rigid_body2: m
                            .data
                            .rigid_body2
                            .and_then(|h| self.bodies.map.value_of(&h).cloned())
                            .unwrap_or_default(),
                        normal: m.data.normal,
                    })
                    .collect(),
                has_any_active_contact: c.has_any_active_contact,
            })
    }
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for PhysicsWorld {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "PhysicsWorld")
    }
}
