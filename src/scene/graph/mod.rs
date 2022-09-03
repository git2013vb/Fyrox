//! Contains all methods and structures to create and manage scene graphs.
//!
//! Scene graph is the foundation of the engine. Graph is a hierarchical data
//! structure where each element called node. Each node can have zero to one parent
//! node, and any children nodes. Node with no parent node called root, with no
//! children nodes - leaf. Graphical representation can be something like this:
//!
//! ```text
//!     Root____
//!       |    |
//!       D    A___
//!       |    |  |
//!       E    C  B
//!     ............
//! ```
//!
//! This picture clearly shows relations between nodes. Such structure allows us
//! to create scenes of any complexity by just linking nodes with each other.
//! Connections between nodes are used to traverse tree, to calculate global
//! transforms, global visibility and many other things. Most interesting here -
//! is global transform calculation - it allows you to produce complex movements
//! just by linking nodes to each other. Good example of this is skeleton which
//! is used in skinning (animating 3d model by set of bones).

use crate::{
    asset::ResourceState,
    core::{
        algebra::{Matrix4, Rotation3, UnitQuaternion, Vector2, Vector3},
        inspect::{Inspect, PropertyInfo},
        instant,
        math::Matrix4Ext,
        pool::{Handle, Pool, Ticket},
        reflect::Reflect,
        visitor::{Visit, VisitResult, Visitor},
    },
    resource::model::{Model, NodeMapping},
    scene::{
        self,
        base::ScriptMessage,
        camera::Camera,
        dim2::{self},
        graph::{
            event::{GraphEvent, GraphEventBroadcaster},
            map::NodeHandleMap,
            physics::{PhysicsPerformanceStatistics, PhysicsWorld},
        },
        mesh::Mesh,
        node::{container::NodeContainer, Node, SyncContext, UpdateContext},
        pivot::Pivot,
        sound::context::SoundContext,
        transform::TransformBuilder,
    },
    utils::log::{Log, MessageKind},
};
use rapier3d::geometry::ColliderHandle;
use std::{
    fmt::Debug,
    ops::{Index, IndexMut},
    sync::mpsc::{channel, Receiver, Sender},
    time::Duration,
};

pub mod event;
pub mod map;
pub mod physics;

/// Graph performance statistics. Allows you to find out "hot" parts of the scene graph, which
/// parts takes the most time to update.
#[derive(Clone, Default, Debug)]
pub struct GraphPerformanceStatistics {
    /// Amount of time that was needed to update global transform, visibility, and every other
    /// property of every object which depends on the state of a parent node.
    pub hierarchical_properties_time: Duration,

    /// Amount of time that was needed to synchronize state of the graph with the state of
    /// backing native objects (Rapier's rigid bodies, colliders, joints, sound sources, etc.)
    pub sync_time: Duration,

    /// Physics performance statistics.
    pub physics: PhysicsPerformanceStatistics,

    /// 2D Physics performance statistics.
    pub physics2d: PhysicsPerformanceStatistics,

    /// A time which was required to render sounds.
    pub sound_update_time: Duration,
}

impl GraphPerformanceStatistics {
    /// Returns total amount of time.
    pub fn total(&self) -> Duration {
        self.hierarchical_properties_time
            + self.sync_time
            + self.physics.total()
            + self.physics2d.total()
            + self.sound_update_time
    }
}

/// A helper type alias for node pool.
pub type NodePool = Pool<Node, NodeContainer>;

/// See module docs.
#[derive(Debug, Inspect, Reflect)]
pub struct Graph {
    #[inspect(skip)]
    #[reflect(hidden)]
    root: Handle<Node>,

    #[inspect(skip)]
    #[reflect(hidden)]
    pool: NodePool,

    #[inspect(skip)]
    #[reflect(hidden)]
    stack: Vec<Handle<Node>>,

    /// Backing physics "world". It is responsible for the physics simulation.
    pub physics: PhysicsWorld,

    /// Backing 2D physics "world". It is responsible for the 2D physics simulation.
    pub physics2d: dim2::physics::PhysicsWorld,

    /// Backing sound context. It is responsible for sound rendering.
    pub sound_context: SoundContext,

    /// Performance statistics of a last [`Graph::update`] call.
    #[inspect(skip)]
    #[reflect(hidden)]
    pub performance_statistics: GraphPerformanceStatistics,

    /// Allows you to "subscribe" for graph events.
    #[reflect(hidden)]
    pub event_broadcaster: GraphEventBroadcaster,

    #[reflect(hidden)]
    pub(crate) script_message_sender: Sender<ScriptMessage>,
    #[reflect(hidden)]
    pub(crate) script_message_receiver: Receiver<ScriptMessage>,
}

impl Default for Graph {
    fn default() -> Self {
        let (tx, rx) = channel();

        Self {
            physics: PhysicsWorld::new(),
            physics2d: dim2::physics::PhysicsWorld::new(),
            root: Handle::NONE,
            pool: Pool::new(),
            stack: Vec::new(),
            sound_context: Default::default(),
            performance_statistics: Default::default(),
            event_broadcaster: Default::default(),
            script_message_receiver: rx,
            script_message_sender: tx,
        }
    }
}

/// Sub-graph is a piece of graph that was extracted from a graph. It has ownership
/// over its nodes. It is used to temporarily take ownership of a sub-graph. This could
/// be used if you making a scene editor with a command stack - once you reverted a command,
/// that created a complex nodes hierarchy (for example you loaded a model) you must store
/// all added nodes somewhere to be able put nodes back into graph when user decide to re-do
/// command. Sub-graph allows you to do this without invalidating handles to nodes.
#[derive(Debug)]
pub struct SubGraph {
    /// A root node and its [ticket](/fyrox-core/model/struct.Ticket.html).
    pub root: (Ticket<Node>, Node),

    /// A set of descendant nodes with their tickets.
    pub descendants: Vec<(Ticket<Node>, Node)>,
}

fn remap_handles(old_new_mapping: &NodeHandleMap, dest_graph: &mut Graph) {
    // Iterate over instantiated nodes and remap handles.
    for (_, &new_node_handle) in old_new_mapping.inner().iter() {
        let dest_node = &mut dest_graph.pool[new_node_handle];
        dest_node.remap_handles(old_new_mapping);

        if let Some(script) = dest_node.script.as_mut() {
            script.remap_handles(old_new_mapping);
        }
    }

    dest_graph.sound_context.remap_handles(old_new_mapping);
}

fn isometric_local_transform(nodes: &NodePool, node: Handle<Node>) -> Matrix4<f32> {
    let transform = nodes[node].local_transform();
    TransformBuilder::new()
        .with_local_position(**transform.position())
        .with_local_rotation(**transform.rotation())
        .with_pre_rotation(**transform.pre_rotation())
        .with_post_rotation(**transform.post_rotation())
        .build()
        .matrix()
}

fn isometric_global_transform(nodes: &NodePool, node: Handle<Node>) -> Matrix4<f32> {
    let parent = nodes[node].parent();
    if parent.is_some() {
        isometric_global_transform(nodes, parent) * isometric_local_transform(nodes, node)
    } else {
        isometric_local_transform(nodes, node)
    }
}

impl Graph {
    /// Creates new graph instance with single root node.
    pub fn new() -> Self {
        let (tx, rx) = channel();
        let mut pool = Pool::new();
        let mut root = Node::new(Pivot::default());
        root.set_name("__ROOT__");
        let root = pool.spawn(root);
        Self {
            physics: Default::default(),
            stack: Vec::new(),
            root,
            pool,
            physics2d: Default::default(),
            sound_context: SoundContext::new(),
            performance_statistics: Default::default(),
            event_broadcaster: Default::default(),
            script_message_receiver: rx,
            script_message_sender: tx,
        }
    }

    /// Adds new node to the graph. Node will be transferred into implementation-defined
    /// storage and you'll get a handle to the node. Node will be automatically attached
    /// to root node of graph, it is required because graph can contain only one root.
    #[inline]
    pub fn add_node(&mut self, mut node: Node) -> Handle<Node> {
        let children = node.children.clone();
        node.children.clear();
        let handle = self.pool.spawn(node);
        if self.root.is_some() {
            self.link_nodes(handle, self.root);
        }
        for child in children {
            self.link_nodes(child, handle);
        }

        self.event_broadcaster.broadcast(GraphEvent::Added(handle));

        let sender = self.script_message_sender.clone();
        let node = &mut self[handle];
        node.self_handle = handle;
        node.script_message_sender = Some(sender);

        handle
    }

    /// Tries to borrow mutable references to two nodes at the same time by given handles. Will
    /// panic if handles overlaps (points to same node).
    pub fn get_two_mut(&mut self, nodes: (Handle<Node>, Handle<Node>)) -> (&mut Node, &mut Node) {
        self.pool.borrow_two_mut(nodes)
    }

    /// Tries to borrow mutable references to three nodes at the same time by given handles. Will
    /// return Err of handles overlaps (points to same node).
    pub fn get_three_mut(
        &mut self,
        nodes: (Handle<Node>, Handle<Node>, Handle<Node>),
    ) -> (&mut Node, &mut Node, &mut Node) {
        self.pool.borrow_three_mut(nodes)
    }

    /// Tries to borrow mutable references to four nodes at the same time by given handles. Will
    /// panic if handles overlaps (points to same node).
    pub fn get_four_mut(
        &mut self,
        nodes: (Handle<Node>, Handle<Node>, Handle<Node>, Handle<Node>),
    ) -> (&mut Node, &mut Node, &mut Node, &mut Node) {
        self.pool.borrow_four_mut(nodes)
    }

    /// Returns root node of current graph.
    pub fn get_root(&self) -> Handle<Node> {
        self.root
    }

    /// Tries to borrow a node, returns Some(node) if the handle is valid, None - otherwise.
    pub fn try_get(&self, handle: Handle<Node>) -> Option<&Node> {
        self.pool.try_borrow(handle)
    }

    /// Tries to mutably borrow a node, returns Some(node) if the handle is valid, None - otherwise.
    pub fn try_get_mut(&mut self, handle: Handle<Node>) -> Option<&mut Node> {
        self.pool.try_borrow_mut(handle)
    }

    /// Destroys node and its children recursively.
    ///
    /// # Notes
    ///
    /// This method does not remove references to the node in other places like animations,
    /// physics, etc. You should prefer to use [Scene::remove_node](crate::scene::Scene::remove_node) -
    /// it automatically breaks all associations between nodes.
    #[inline]
    pub fn remove_node(&mut self, node_handle: Handle<Node>) {
        self.unlink_internal(node_handle);

        self.stack.clear();
        self.stack.push(node_handle);
        while let Some(handle) = self.stack.pop() {
            for &child in self.pool[handle].children().iter() {
                self.stack.push(child);
            }

            // Remove associated entities.
            let mut node = self.pool.free(handle);
            self.clean_up_for_node(&mut node);

            self.event_broadcaster
                .broadcast(GraphEvent::Removed(handle));
        }
    }

    fn clean_up_for_node(&mut self, node: &mut Node) {
        node.clean_up(self);
    }

    fn unlink_internal(&mut self, node_handle: Handle<Node>) {
        // Replace parent handle of child
        let parent_handle = std::mem::replace(&mut self.pool[node_handle].parent, Handle::NONE);

        // Remove child from parent's children list
        if parent_handle.is_some() {
            let parent = &mut self.pool[parent_handle];
            if let Some(i) = parent.children().iter().position(|h| *h == node_handle) {
                parent.children.remove(i);
            }
        }

        let node_ref = &mut self.pool[node_handle];

        // Remove native collider when detaching a collider node from rigid body node.
        if let Some(collider) = node_ref.cast_mut::<scene::collider::Collider>() {
            if self.physics.remove_collider(collider.native.get()) {
                collider.native.set(ColliderHandle::invalid());
            }
        } else if let Some(collider2d) = node_ref.cast_mut::<dim2::collider::Collider>() {
            if self.physics2d.remove_collider(collider2d.native.get()) {
                collider2d
                    .native
                    .set(rapier2d::geometry::ColliderHandle::invalid());
            }
        }
    }

    /// Links specified child with specified parent.
    #[inline]
    pub fn link_nodes(&mut self, child: Handle<Node>, parent: Handle<Node>) {
        self.unlink_internal(child);
        self.pool[child].parent = parent;
        self.pool[parent].children.push(child);
    }

    /// Unlinks specified node from its parent and attaches it to root graph node.
    #[inline]
    pub fn unlink_node(&mut self, node_handle: Handle<Node>) {
        self.unlink_internal(node_handle);
        self.link_nodes(node_handle, self.root);
        self.pool[node_handle]
            .local_transform_mut()
            .set_position(Vector3::default());
    }

    /// Tries to find a copy of `node_handle` in hierarchy tree starting from `root_handle`.
    pub fn find_copy_of(
        &self,
        root_handle: Handle<Node>,
        node_handle: Handle<Node>,
    ) -> Handle<Node> {
        let root = &self.pool[root_handle];
        if root.original_handle_in_resource() == node_handle {
            return root_handle;
        }

        for child_handle in root.children() {
            let out = self.find_copy_of(*child_handle, node_handle);
            if out.is_some() {
                return out;
            }
        }

        Handle::NONE
    }

    /// Searches node using specified compare closure starting from specified node. If nothing
    /// was found [`Handle::NONE`] is returned.
    pub fn find<C>(&self, root_node: Handle<Node>, cmp: &mut C) -> Handle<Node>
    where
        C: FnMut(&Node) -> bool,
    {
        let root = &self.pool[root_node];
        if cmp(root) {
            root_node
        } else {
            let mut result: Handle<Node> = Handle::NONE;
            for child in root.children() {
                let child_handle = self.find(*child, cmp);
                if !child_handle.is_none() {
                    result = child_handle;
                    break;
                }
            }
            result
        }
    }

    /// Searches node with specified name starting from specified node. If nothing was found,
    /// [`Handle::NONE`] is returned.
    pub fn find_by_name(&self, root_node: Handle<Node>, name: &str) -> Handle<Node> {
        self.find(root_node, &mut |node| node.name() == name)
    }

    /// Searches node with specified name starting from root. If nothing was found, `Handle::NONE`
    /// is returned.
    pub fn find_by_name_from_root(&self, name: &str) -> Handle<Node> {
        self.find_by_name(self.root, name)
    }

    /// Searches node using specified compare closure starting from root. If nothing was found,
    /// `Handle::NONE` is returned.
    pub fn find_from_root<C>(&self, cmp: &mut C) -> Handle<Node>
    where
        C: FnMut(&Node) -> bool,
    {
        self.find(self.root, cmp)
    }

    /// Creates deep copy of node with all children. This is relatively heavy operation!
    /// In case if any error happened it returns `Handle::NONE`. This method can be used
    /// to create exact copy of given node hierarchy. For example you can prepare rocket
    /// model: case of rocket will be mesh, and fire from nozzle will be particle system,
    /// and when you fire from rocket launcher you just need to create a copy of such
    /// "prefab".
    ///
    /// # Notes
    ///
    /// This method does *not* copy any animations! You have to copy them manually. In most
    /// cases it is fine to retarget animation from a resource you want, it will create
    /// animation copy from resource that will work with your nodes hierarchy.
    ///
    /// # Implementation notes
    ///
    /// This method automatically remaps bones for copied surfaces.
    ///
    /// Returns tuple where first element is handle to copy of node, and second element -
    /// old-to-new hash map, which can be used to easily find copy of node by its original.
    ///
    /// Filter allows to exclude some nodes from copied hierarchy. It must return false for
    /// odd nodes. Filtering applied only to descendant nodes.
    pub fn copy_node<F>(
        &self,
        node_handle: Handle<Node>,
        dest_graph: &mut Graph,
        filter: &mut F,
    ) -> (Handle<Node>, NodeHandleMap)
    where
        F: FnMut(Handle<Node>, &Node) -> bool,
    {
        let mut old_new_mapping = NodeHandleMap::default();
        let root_handle = self.copy_node_raw(node_handle, dest_graph, &mut old_new_mapping, filter);

        remap_handles(&old_new_mapping, dest_graph);

        (root_handle, old_new_mapping)
    }

    /// Creates deep copy of node with all children. This is relatively heavy operation!
    /// In case if any error happened it returns `Handle::NONE`. This method can be used
    /// to create exact copy of given node hierarchy. For example you can prepare rocket
    /// model: case of rocket will be mesh, and fire from nozzle will be particle system,
    /// and when you fire from rocket launcher you just need to create a copy of such
    /// "prefab".
    ///
    /// # Notes
    ///
    /// This method has exactly the same functionality as `copy_node`, but copies not in-place.
    /// This method does *not* copy any animations! You have to copy them manually. In most
    /// cases it is fine to retarget animation from a resource you want, it will create
    /// animation copy from resource that will work with your nodes hierarchy.
    ///
    /// # Implementation notes
    ///
    /// This method automatically remaps bones for copied surfaces.
    ///
    /// Returns tuple where first element is handle to copy of node, and second element -
    /// old-to-new hash map, which can be used to easily find copy of node by its original.
    ///
    /// Filter allows to exclude some nodes from copied hierarchy. It must return false for
    /// odd nodes. Filtering applied only to descendant nodes.
    pub fn copy_node_inplace<F>(
        &mut self,
        node_handle: Handle<Node>,
        filter: &mut F,
    ) -> (Handle<Node>, NodeHandleMap)
    where
        F: FnMut(Handle<Node>, &Node) -> bool,
    {
        let mut old_new_mapping = NodeHandleMap::default();

        let to_copy = self
            .traverse_handle_iter(node_handle)
            .map(|node| (node, self.pool[node].children.clone()))
            .collect::<Vec<_>>();

        let mut root_handle = Handle::NONE;

        for (parent, children) in to_copy.iter() {
            // Copy parent first.
            let parent_copy = self.pool[*parent].clone_box();
            let parent_copy_handle = self.add_node(parent_copy);
            old_new_mapping.map.insert(*parent, parent_copy_handle);

            if root_handle.is_none() {
                root_handle = parent_copy_handle;
            }

            // Copy children and link to new parent.
            for &child in children {
                if filter(child, &self.pool[child]) {
                    let child_copy = self.pool[child].clone_box();
                    let child_copy_handle = self.add_node(child_copy);
                    old_new_mapping.map.insert(child, child_copy_handle);
                    self.link_nodes(child_copy_handle, parent_copy_handle);
                }
            }
        }

        remap_handles(&old_new_mapping, self);

        (root_handle, old_new_mapping)
    }

    /// Creates copy of a node and breaks all connections with other nodes. Keep in mind that
    /// this method may give unexpected results when the node has connections with other nodes.
    /// For example if you'll try to copy a skinned mesh, its copy won't be skinned anymore -
    /// you'll get just a "shallow" mesh. Also unlike [copy_node](struct.Graph.html#method.copy_node)
    /// this method returns copied node directly, it does not inserts it in any graph.
    pub fn copy_single_node(&self, node_handle: Handle<Node>) -> Node {
        let node = &self.pool[node_handle];
        let mut clone = node.clone_box();
        clone.parent = Handle::NONE;
        clone.children.clear();
        if let Some(ref mut mesh) = clone.cast_mut::<Mesh>() {
            for surface in mesh.surfaces_mut() {
                surface.bones.clear();
            }
        }
        clone
    }

    fn copy_node_raw<F>(
        &self,
        root_handle: Handle<Node>,
        dest_graph: &mut Graph,
        old_new_mapping: &mut NodeHandleMap,
        filter: &mut F,
    ) -> Handle<Node>
    where
        F: FnMut(Handle<Node>, &Node) -> bool,
    {
        let src_node = &self.pool[root_handle];
        let dest_node = src_node.clone_box();
        let dest_copy_handle = dest_graph.add_node(dest_node);
        old_new_mapping.map.insert(root_handle, dest_copy_handle);
        for &src_child_handle in src_node.children() {
            if filter(src_child_handle, &self.pool[src_child_handle]) {
                let dest_child_handle =
                    self.copy_node_raw(src_child_handle, dest_graph, old_new_mapping, filter);
                if !dest_child_handle.is_none() {
                    dest_graph.link_nodes(dest_child_handle, dest_copy_handle);
                }
            }
        }
        dest_copy_handle
    }

    fn restore_original_handles(&mut self) {
        // Iterate over each node in the graph and resolve original handles. Original handle is a handle
        // to a node in resource from which a node was instantiated from. Also sync templated properties
        // if needed and copy surfaces from originals.
        for node in self.pool.iter_mut() {
            if let Some(model) = node.resource() {
                let model = model.state();
                match *model {
                    ResourceState::Ok(ref data) => {
                        let resource_graph = &data.get_scene().graph;

                        let resource_node = match data.mapping {
                            NodeMapping::UseNames => {
                                // For some models we can resolve it only by names of nodes, but this is not
                                // reliable way of doing this, because some editors allow nodes to have same
                                // names for objects, but here we'll assume that modellers will not create
                                // models with duplicated names and user of the engine reads log messages.
                                resource_graph
                                    .pair_iter()
                                    .find_map(|(handle, resource_node)| {
                                        if resource_node.name() == node.name() {
                                            Some((resource_node, handle))
                                        } else {
                                            None
                                        }
                                    })
                            }
                            NodeMapping::UseHandles => {
                                // Use original handle directly.
                                resource_graph
                                    .pool
                                    .try_borrow(node.original_handle_in_resource)
                                    .map(|resource_node| {
                                        (resource_node, node.original_handle_in_resource)
                                    })
                            }
                        };

                        if let Some((resource_node, original)) = resource_node {
                            node.original_handle_in_resource = original;
                            node.inv_bind_pose_transform = resource_node.inv_bind_pose_transform();

                            Log::verify(node.inherit(resource_node));
                        } else {
                            Log::warn(format!(
                                "Unable to find original handle for node {}",
                                node.name(),
                            ))
                        }
                    }
                    ResourceState::Pending { .. } => {
                        panic!("resources must be awaited before doing resolve!")
                    }
                    _ => {}
                }
            }
        }

        Log::writeln(MessageKind::Information, "Original handles resolved!");
    }

    fn remap_handles(&mut self, instances: &[(Handle<Node>, Model)]) {
        for (instance_root, resource) in instances {
            // Prepare old -> new handle mapping first by walking over the graph
            // starting from instance root.
            let mut old_new_mapping = NodeHandleMap::default();
            let mut traverse_stack = vec![*instance_root];
            while let Some(node_handle) = traverse_stack.pop() {
                let node = &self.pool[node_handle];
                if let Some(node_resource) = node.resource().as_ref() {
                    // We're interested only in instance nodes.
                    if node_resource == resource {
                        let previous_mapping = old_new_mapping
                            .map
                            .insert(node.original_handle_in_resource, node_handle);
                        // There should be no such node.
                        if previous_mapping.is_some() {
                            Log::warn(format!(
                                "There are multiple original nodes for {:?}! Previous was {:?}. \
                                This can happen if a respective node was deleted.",
                                node_handle, node.original_handle_in_resource
                            ))
                        }
                    }
                }

                traverse_stack.extend_from_slice(node.children());
            }

            // Lastly, remap handles. We can't do this in single pass because there could
            // be cross references.
            for (_, handle) in old_new_mapping.map.iter() {
                self.pool[*handle].remap_handles(&old_new_mapping);
            }
        }
    }

    fn restore_integrity(&mut self) -> Vec<(Handle<Node>, Model)> {
        Log::writeln(MessageKind::Information, "Checking integrity...");

        // Check integrity - if a node was added in resource, it must be also added in the graph.
        // However if a node was deleted in resource, we must leave it the graph because there
        // might be some other nodes that were attached to the one that was deleted in resource or
        // a node might be referenced somewhere in user code.
        let instances = self
            .pool
            .pair_iter()
            .filter_map(|(h, n)| {
                if n.is_resource_instance_root {
                    Some((h, n.resource().unwrap()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let instance_count = instances.len();
        let mut restored_count = 0;

        for (instance_root, resource) in instances.iter().cloned() {
            let model = resource.state();
            if let ResourceState::Ok(ref data) = *model {
                let resource_graph = &data.get_scene().graph;

                let resource_instance_root = self.pool[instance_root].original_handle_in_resource;

                if resource_instance_root.is_none() {
                    let instance = &self.pool[instance_root];
                    Log::writeln(
                        MessageKind::Warning,
                        format!(
                            "There is an instance of resource {} \
                    but original node {} cannot be found!",
                            data.path.display(),
                            instance.name()
                        ),
                    );

                    continue;
                }

                let mut traverse_stack = vec![resource_instance_root];
                while let Some(resource_node_handle) = traverse_stack.pop() {
                    let resource_node = &resource_graph[resource_node_handle];

                    // Root of the resource is not belongs to resource, it is just a convenient way of
                    // consolidation all descendants under a single node.
                    let mut compare =
                        |n: &Node| n.original_handle_in_resource == resource_node_handle;

                    if resource_node_handle != resource_graph.root
                        && self.find(instance_root, &mut compare).is_none()
                    {
                        Log::writeln(
                            MessageKind::Warning,
                            format!(
                                "Instance of node {} is missing. Restoring integrity...",
                                resource_node.name()
                            ),
                        );

                        // Instantiate missing node.
                        let (copy, old_to_new_mapping) = Model::instantiate_from(
                            resource.clone(),
                            data,
                            resource_node_handle,
                            self,
                        );

                        restored_count += old_to_new_mapping.map.len();

                        // Link it with existing node.
                        if resource_node.parent().is_some() {
                            let parent = self.find(instance_root, &mut |n| {
                                n.original_handle_in_resource == resource_node.parent()
                            });

                            if parent.is_some() {
                                self.link_nodes(copy, parent);
                            } else {
                                // Fail-safe route - link with root of instance.
                                self.link_nodes(copy, instance_root);
                            }
                        } else {
                            // Fail-safe route - link with root of instance.
                            self.link_nodes(copy, instance_root);
                        }
                    }

                    traverse_stack.extend_from_slice(resource_node.children());
                }
            }
        }

        Log::writeln(
            MessageKind::Information,
            format!(
                "Integrity restored for {} instances! {} new nodes were added!",
                instance_count, restored_count
            ),
        );

        instances
    }

    fn restore_dynamic_node_data(&mut self) {
        for (handle, node) in self.pool.pair_iter_mut() {
            node.self_handle = handle;
            node.script_message_sender = Some(self.script_message_sender.clone());
        }
    }

    pub(crate) fn resolve(&mut self) {
        Log::writeln(MessageKind::Information, "Resolving graph...");

        self.restore_dynamic_node_data();
        self.update_hierarchical_data();
        self.restore_original_handles();
        let instances = self.restore_integrity();
        self.remap_handles(&instances);

        // Update cube maps for sky boxes.
        for node in self.linear_iter_mut() {
            if let Some(camera) = node.cast_mut::<Camera>() {
                if let Some(skybox) = camera.skybox_mut() {
                    Log::verify(skybox.create_cubemap());
                }
            }
        }

        Log::writeln(MessageKind::Information, "Graph resolved successfully!");
    }

    /// Calculates local and global transform, global visibility for each node in graph.
    /// Normally you not need to call this method directly, it will be called automatically
    /// on each frame. However there is one use case - when you setup complex hierarchy and
    /// need to know global transform of nodes before entering update loop, then you can call
    /// this method.
    pub fn update_hierarchical_data(&mut self) {
        fn update_recursively(
            nodes: &NodePool,
            sound_context: &mut SoundContext,
            physics: &mut PhysicsWorld,
            physics2d: &mut dim2::physics::PhysicsWorld,
            node_handle: Handle<Node>,
        ) {
            let node = &nodes[node_handle];

            let (parent_global_transform, parent_visibility) =
                if let Some(parent) = nodes.try_borrow(node.parent()) {
                    (parent.global_transform(), parent.global_visibility())
                } else {
                    (Matrix4::identity(), true)
                };

            let new_global_transform = parent_global_transform * node.local_transform().matrix();

            // TODO: Detect changes from user code here.
            node.sync_transform(
                &new_global_transform,
                &mut SyncContext {
                    nodes,
                    physics,
                    physics2d,
                    sound_context,
                },
            );

            node.global_transform.set(new_global_transform);
            node.global_visibility
                .set(parent_visibility && node.visibility());

            for &child in node.children() {
                update_recursively(nodes, sound_context, physics, physics2d, child);
            }
        }

        update_recursively(
            &self.pool,
            &mut self.sound_context,
            &mut self.physics,
            &mut self.physics2d,
            self.root,
        );
    }

    /// Checks whether given node handle is valid or not.
    pub fn is_valid_handle(&self, node_handle: Handle<Node>) -> bool {
        self.pool.is_valid_handle(node_handle)
    }

    fn sync_native(&mut self) {
        let mut sync_context = SyncContext {
            nodes: &self.pool,
            physics: &mut self.physics,
            physics2d: &mut self.physics2d,
            sound_context: &mut self.sound_context,
        };

        for (handle, node) in self.pool.pair_iter() {
            node.sync_native(handle, &mut sync_context);
        }
    }

    /// Updates nodes in graph using given delta time. There is no need to call it manually.
    pub fn update(&mut self, frame_size: Vector2<f32>, dt: f32) {
        let last_time = instant::Instant::now();
        self.update_hierarchical_data();
        self.performance_statistics.hierarchical_properties_time =
            instant::Instant::now() - last_time;

        let last_time = instant::Instant::now();
        self.sync_native();
        self.performance_statistics.sync_time = instant::Instant::now() - last_time;

        self.physics.performance_statistics.reset();
        self.physics.update();
        self.performance_statistics.physics = self.physics.performance_statistics.clone();

        self.physics2d.performance_statistics.reset();
        self.physics2d.update();
        self.performance_statistics.physics2d = self.physics2d.performance_statistics.clone();

        self.sound_context.update(&self.pool);
        self.performance_statistics.sound_update_time = self.sound_context.full_render_duration();

        for i in 0..self.pool.get_capacity() {
            let mut update_context = UpdateContext {
                frame_size,
                dt,
                // SAFETY: There multiple reasons why this is safe to get immutable reference to nodes
                // along with mutable reference:
                //
                // 1) `Pool` uses indexes to reference data, any internal buffer reallocation in the
                //    pool will **not** invalidate anything.
                // 2) Internal pool reallocation is not possible, because use it for mutable iteration,
                //    and does **not** allow any other code to call dangerous methods, because second
                //    reference is immutable.
                // 3) `Pool::free` does not cause any memory reallocation, so pointers in pool iterators
                //    will be valid.
                // 4) There is no multithreading in update calls, so no data races.
                nodes: unsafe { &(*(self as *const Graph)).pool },
                physics: &mut self.physics,
                physics2d: &mut self.physics2d,
                sound_context: &mut self.sound_context,
            };

            let handle = self.pool.handle_from_index(i);
            if let Some(node) = self.pool.at_mut(i) {
                node.transform_modified.set(false);

                let is_alive = node.update(&mut update_context);

                if !is_alive {
                    self.remove_node(handle);
                }
            }
        }
    }

    /// Returns capacity of internal pool. Can be used to iterate over all **potentially**
    /// available indices and try to convert them to handles.
    ///
    /// ```
    /// use fyrox::scene::node::Node;
    /// use fyrox::scene::graph::Graph;
    /// use fyrox::scene::pivot::Pivot;
    /// let mut graph = Graph::new();
    /// graph.add_node(Node::new(Pivot::default()));
    /// graph.add_node(Node::new(Pivot::default()));
    /// for i in 0..graph.capacity() {
    ///     let handle = graph.handle_from_index(i);
    ///     if handle.is_some() {
    ///         let node = &mut graph[handle];
    ///         // Do something with node.
    ///     }
    /// }
    /// ```
    pub fn capacity(&self) -> u32 {
        self.pool.get_capacity()
    }

    /// Makes new handle from given index. Handle will be none if index was either out-of-bounds
    /// or point to a vacant pool entry.
    ///
    /// ```
    /// use fyrox::scene::node::Node;
    /// use fyrox::scene::graph::Graph;
    /// use fyrox::scene::pivot::Pivot;
    /// let mut graph = Graph::new();
    /// graph.add_node(Node::new(Pivot::default()));
    /// graph.add_node(Node::new(Pivot::default()));
    /// for i in 0..graph.capacity() {
    ///     let handle = graph.handle_from_index(i);
    ///     if handle.is_some() {
    ///         let node = &mut graph[handle];
    ///         // Do something with node.
    ///     }
    /// }
    /// ```
    pub fn handle_from_index(&self, index: u32) -> Handle<Node> {
        self.pool.handle_from_index(index)
    }

    /// Creates an iterator that has linear iteration order over internal collection
    /// of nodes. It does *not* perform any tree traversal!
    pub fn linear_iter(&self) -> impl Iterator<Item = &Node> {
        self.pool.iter()
    }

    /// Creates an iterator that has linear iteration order over internal collection
    /// of nodes. It does *not* perform any tree traversal!
    pub fn linear_iter_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.pool.iter_mut()
    }

    /// Creates new iterator that iterates over internal collection giving (handle; node) pairs.
    pub fn pair_iter(&self) -> impl Iterator<Item = (Handle<Node>, &Node)> {
        self.pool.pair_iter()
    }

    /// Creates new iterator that iterates over internal collection giving (handle; node) pairs.
    pub fn pair_iter_mut(&mut self) -> impl Iterator<Item = (Handle<Node>, &mut Node)> {
        self.pool.pair_iter_mut()
    }

    /// Extracts node from graph and reserves its handle. It is used to temporarily take
    /// ownership over node, and then put node back using given ticket. Extracted node is
    /// detached from its parent!
    pub fn take_reserve(&mut self, handle: Handle<Node>) -> (Ticket<Node>, Node) {
        self.unlink_internal(handle);
        self.take_reserve_internal(handle)
    }

    pub(crate) fn take_reserve_internal(&mut self, handle: Handle<Node>) -> (Ticket<Node>, Node) {
        self.pool.take_reserve(handle)
    }

    /// Puts node back by given ticket. Attaches back to root node of graph.
    pub fn put_back(&mut self, ticket: Ticket<Node>, node: Node) -> Handle<Node> {
        let handle = self.put_back_internal(ticket, node);
        self.link_nodes(handle, self.root);
        handle
    }

    pub(crate) fn put_back_internal(&mut self, ticket: Ticket<Node>, node: Node) -> Handle<Node> {
        self.pool.put_back(ticket, node)
    }

    /// Makes node handle vacant again.
    pub fn forget_ticket(&mut self, ticket: Ticket<Node>, mut node: Node) -> Node {
        self.pool.forget_ticket(ticket);
        self.clean_up_for_node(&mut node);
        node
    }

    /// Extracts sub-graph starting from a given node. All handles to extracted nodes
    /// becomes reserved and will be marked as "occupied", an attempt to borrow a node
    /// at such handle will result in panic!. Please note that root node will be
    /// detached from its parent!
    pub fn take_reserve_sub_graph(&mut self, root: Handle<Node>) -> SubGraph {
        // Take out descendants first.
        let mut descendants = Vec::new();
        let mut stack = self[root].children().to_vec();
        while let Some(handle) = stack.pop() {
            stack.extend_from_slice(self[handle].children());
            descendants.push(self.pool.take_reserve(handle));
        }

        SubGraph {
            // Root must be extracted with detachment from its parent (if any).
            root: self.take_reserve(root),
            descendants,
        }
    }

    /// Puts previously extracted sub-graph into graph. Handles to nodes will become valid
    /// again. After that you probably want to re-link returned handle with its previous
    /// parent.
    pub fn put_sub_graph_back(&mut self, sub_graph: SubGraph) -> Handle<Node> {
        for (ticket, node) in sub_graph.descendants {
            self.pool.put_back(ticket, node);
        }

        let (ticket, node) = sub_graph.root;
        let root_handle = self.put_back(ticket, node);

        self.link_nodes(root_handle, self.root);

        root_handle
    }

    /// Forgets the entire sub-graph making handles to nodes invalid.
    pub fn forget_sub_graph(&mut self, sub_graph: SubGraph) {
        for (ticket, mut node) in sub_graph.descendants {
            self.pool.forget_ticket(ticket);
            self.clean_up_for_node(&mut node);
        }
        let (ticket, mut root) = sub_graph.root;
        self.pool.forget_ticket(ticket);
        self.clean_up_for_node(&mut root);
    }

    /// Returns the number of nodes in the graph.
    pub fn node_count(&self) -> u32 {
        self.pool.alive_count()
    }

    /// Create a graph depth traversal iterator.
    ///
    /// # Notes
    ///
    /// This method allocates temporal array so it is not cheap! Should not be
    /// used on each frame.
    pub fn traverse_iter(&self, from: Handle<Node>) -> GraphTraverseIterator {
        GraphTraverseIterator {
            graph: self,
            stack: vec![from],
        }
    }

    /// Create a graph depth traversal iterator which will emit *handles* to nodes.
    ///
    /// # Notes
    ///
    /// This method allocates temporal array so it is not cheap! Should not be
    /// used on each frame.
    pub fn traverse_handle_iter(&self, from: Handle<Node>) -> GraphHandleTraverseIterator {
        GraphHandleTraverseIterator {
            graph: self,
            stack: vec![from],
        }
    }

    /// Creates deep copy of graph. Allows filtering while copying, returns copy and
    /// old-to-new node mapping.
    pub fn clone<F>(&self, filter: &mut F) -> (Self, NodeHandleMap)
    where
        F: FnMut(Handle<Node>, &Node) -> bool,
    {
        let mut copy = Self::default();
        let (root, old_new_map) = self.copy_node(self.root, &mut copy, filter);
        copy.root = root;
        (copy, old_new_map)
    }

    /// Returns local transformation matrix of a node without scale.
    pub fn local_transform_no_scale(&self, node: Handle<Node>) -> Matrix4<f32> {
        let mut transform = self[node].local_transform().clone();
        transform.set_scale(Vector3::new(1.0, 1.0, 1.0));
        transform.matrix()
    }

    /// Returns world transformation matrix of a node without scale.
    pub fn global_transform_no_scale(&self, node: Handle<Node>) -> Matrix4<f32> {
        let parent = self[node].parent();
        if parent.is_some() {
            self.global_transform_no_scale(parent) * self.local_transform_no_scale(node)
        } else {
            self.local_transform_no_scale(node)
        }
    }

    /// Returns isometric local transformation matrix of a node. Such transform has
    /// only translation and rotation.
    pub fn isometric_local_transform(&self, node: Handle<Node>) -> Matrix4<f32> {
        isometric_local_transform(&self.pool, node)
    }

    /// Returns world transformation matrix of a node only.  Such transform has
    /// only translation and rotation.
    pub fn isometric_global_transform(&self, node: Handle<Node>) -> Matrix4<f32> {
        isometric_global_transform(&self.pool, node)
    }

    /// Returns global scale matrix of a node.
    pub fn global_scale_matrix(&self, node: Handle<Node>) -> Matrix4<f32> {
        let node = &self[node];
        let local_scale_matrix = Matrix4::new_nonuniform_scaling(node.local_transform().scale());
        if node.parent().is_some() {
            self.global_scale_matrix(node.parent()) * local_scale_matrix
        } else {
            local_scale_matrix
        }
    }

    /// Returns rotation quaternion of a node in world coordinates.
    pub fn global_rotation(&self, node: Handle<Node>) -> UnitQuaternion<f32> {
        UnitQuaternion::from(Rotation3::from_matrix_eps(
            &self.global_transform_no_scale(node).basis(),
            f32::EPSILON,
            16,
            Rotation3::identity(),
        ))
    }

    /// Returns rotation quaternion of a node in world coordinates without pre- and post-rotations.
    pub fn isometric_global_rotation(&self, node: Handle<Node>) -> UnitQuaternion<f32> {
        UnitQuaternion::from(Rotation3::from_matrix_eps(
            &self.isometric_global_transform(node).basis(),
            f32::EPSILON,
            16,
            Rotation3::identity(),
        ))
    }

    /// Returns rotation quaternion and position of a node in world coordinates, scale is eliminated.
    pub fn global_rotation_position_no_scale(
        &self,
        node: Handle<Node>,
    ) -> (UnitQuaternion<f32>, Vector3<f32>) {
        (self.global_rotation(node), self[node].global_position())
    }

    /// Returns isometric global rotation and position.
    pub fn isometric_global_rotation_position(
        &self,
        node: Handle<Node>,
    ) -> (UnitQuaternion<f32>, Vector3<f32>) {
        (
            self.isometric_global_rotation(node),
            self[node].global_position(),
        )
    }

    /// Returns global scale of a node.
    pub fn global_scale(&self, node: Handle<Node>) -> Vector3<f32> {
        let m = self.global_scale_matrix(node);
        Vector3::new(m[0], m[5], m[10])
    }
}

impl Index<Handle<Node>> for Graph {
    type Output = Node;

    fn index(&self, index: Handle<Node>) -> &Self::Output {
        &self.pool[index]
    }
}

impl IndexMut<Handle<Node>> for Graph {
    fn index_mut(&mut self, index: Handle<Node>) -> &mut Self::Output {
        &mut self.pool[index]
    }
}

/// Iterator that traverses tree in depth and returns shared references to nodes.
pub struct GraphTraverseIterator<'a> {
    graph: &'a Graph,
    stack: Vec<Handle<Node>>,
}

impl<'a> Iterator for GraphTraverseIterator<'a> {
    type Item = &'a Node;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(handle) = self.stack.pop() {
            let node = &self.graph[handle];

            for child_handle in node.children() {
                self.stack.push(*child_handle);
            }

            return Some(node);
        }

        None
    }
}

/// Iterator that traverses tree in depth and returns handles to nodes.
pub struct GraphHandleTraverseIterator<'a> {
    graph: &'a Graph,
    stack: Vec<Handle<Node>>,
}

impl<'a> Iterator for GraphHandleTraverseIterator<'a> {
    type Item = Handle<Node>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(handle) = self.stack.pop() {
            for child_handle in self.graph[handle].children() {
                self.stack.push(*child_handle);
            }

            return Some(handle);
        }
        None
    }
}

impl Visit for Graph {
    fn visit(&mut self, name: &str, visitor: &mut Visitor) -> VisitResult {
        // Pool must be empty, otherwise handles will be invalid and everything will blow up.
        if visitor.is_reading() && self.pool.get_capacity() != 0 {
            panic!("Graph pool must be empty on load!")
        }

        let mut region = visitor.enter_region(name)?;

        self.root.visit("Root", &mut region)?;
        self.pool.visit("Pool", &mut region)?;
        self.sound_context.visit("SoundContext", &mut region)?;
        self.physics.visit("PhysicsWorld", &mut region)?;
        self.physics2d.visit("PhysicsWorld2D", &mut region)?;

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::{
        core::pool::Handle,
        scene::{graph::Graph, node::Node, pivot::Pivot},
    };

    #[test]
    fn graph_init_test() {
        let graph = Graph::new();
        assert_ne!(graph.root, Handle::NONE);
        assert_eq!(graph.pool.alive_count(), 1);
    }

    #[test]
    fn graph_node_test() {
        let mut graph = Graph::new();
        graph.add_node(Node::new(Pivot::default()));
        graph.add_node(Node::new(Pivot::default()));
        graph.add_node(Node::new(Pivot::default()));
        assert_eq!(graph.pool.alive_count(), 4);
    }
}
