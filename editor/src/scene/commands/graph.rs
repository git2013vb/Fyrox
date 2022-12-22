use crate::{command::Command, scene::commands::SceneContext};
use fyrox::{
    core::{
        algebra::{UnitQuaternion, Vector3},
        pool::{Handle, Ticket},
    },
    scene::{
        base::Base,
        graph::{Graph, SubGraph},
        node::Node,
    },
};

#[derive(Debug)]
pub struct MoveNodeCommand {
    node: Handle<Node>,
    old_position: Vector3<f32>,
    new_position: Vector3<f32>,
}

impl MoveNodeCommand {
    pub fn new(node: Handle<Node>, old_position: Vector3<f32>, new_position: Vector3<f32>) -> Self {
        Self {
            node,
            old_position,
            new_position,
        }
    }

    fn swap(&mut self) -> Vector3<f32> {
        let position = self.new_position;
        std::mem::swap(&mut self.new_position, &mut self.old_position);
        position
    }

    fn set_position(&self, graph: &mut Graph, position: Vector3<f32>) {
        graph[self.node]
            .local_transform_mut()
            .set_position(position);
    }
}

impl Command for MoveNodeCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Move Node".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        let position = self.swap();
        self.set_position(&mut context.scene.graph, position);
    }

    fn revert(&mut self, context: &mut SceneContext) {
        let position = self.swap();
        self.set_position(&mut context.scene.graph, position);
    }
}

#[derive(Debug)]
pub struct ScaleNodeCommand {
    node: Handle<Node>,
    old_scale: Vector3<f32>,
    new_scale: Vector3<f32>,
}

impl ScaleNodeCommand {
    pub fn new(node: Handle<Node>, old_scale: Vector3<f32>, new_scale: Vector3<f32>) -> Self {
        Self {
            node,
            old_scale,
            new_scale,
        }
    }

    fn swap(&mut self) -> Vector3<f32> {
        let position = self.new_scale;
        std::mem::swap(&mut self.new_scale, &mut self.old_scale);
        position
    }

    fn set_scale(&self, graph: &mut Graph, scale: Vector3<f32>) {
        graph[self.node].local_transform_mut().set_scale(scale);
    }
}

impl Command for ScaleNodeCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Scale Node".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        let scale = self.swap();
        self.set_scale(&mut context.scene.graph, scale);
    }

    fn revert(&mut self, context: &mut SceneContext) {
        let scale = self.swap();
        self.set_scale(&mut context.scene.graph, scale);
    }
}

#[derive(Debug)]
pub struct RotateNodeCommand {
    node: Handle<Node>,
    old_rotation: UnitQuaternion<f32>,
    new_rotation: UnitQuaternion<f32>,
}

impl RotateNodeCommand {
    pub fn new(
        node: Handle<Node>,
        old_rotation: UnitQuaternion<f32>,
        new_rotation: UnitQuaternion<f32>,
    ) -> Self {
        Self {
            node,
            old_rotation,
            new_rotation,
        }
    }

    fn swap(&mut self) -> UnitQuaternion<f32> {
        let position = self.new_rotation;
        std::mem::swap(&mut self.new_rotation, &mut self.old_rotation);
        position
    }

    fn set_rotation(&self, graph: &mut Graph, rotation: UnitQuaternion<f32>) {
        graph[self.node]
            .local_transform_mut()
            .set_rotation(rotation);
    }
}

impl Command for RotateNodeCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Rotate Node".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        let rotation = self.swap();
        self.set_rotation(&mut context.scene.graph, rotation);
    }

    fn revert(&mut self, context: &mut SceneContext) {
        let rotation = self.swap();
        self.set_rotation(&mut context.scene.graph, rotation);
    }
}

#[derive(Debug)]
pub struct LinkNodesCommand {
    child: Handle<Node>,
    parent: Handle<Node>,
}

impl LinkNodesCommand {
    pub fn new(child: Handle<Node>, parent: Handle<Node>) -> Self {
        Self { child, parent }
    }

    fn link(&mut self, graph: &mut Graph) {
        let old_parent = graph[self.child].parent();
        graph.link_nodes(self.child, self.parent);
        self.parent = old_parent;
    }
}

impl Command for LinkNodesCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Link Nodes".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        self.link(&mut context.scene.graph);
    }

    fn revert(&mut self, context: &mut SceneContext) {
        self.link(&mut context.scene.graph);
    }
}

#[derive(Debug)]
pub struct DeleteNodeCommand {
    handle: Handle<Node>,
    ticket: Option<Ticket<Node>>,
    node: Option<Node>,
    parent: Handle<Node>,
}

impl Command for DeleteNodeCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Delete Node".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        self.parent = context.scene.graph[self.handle].parent();
        let (ticket, node) = context.scene.graph.take_reserve(self.handle);
        self.node = Some(node);
        self.ticket = Some(ticket);
    }

    fn revert(&mut self, context: &mut SceneContext) {
        self.handle = context
            .scene
            .graph
            .put_back(self.ticket.take().unwrap(), self.node.take().unwrap());
        context.scene.graph.link_nodes(self.handle, self.parent);
    }

    fn finalize(&mut self, context: &mut SceneContext) {
        if let Some(ticket) = self.ticket.take() {
            context
                .scene
                .graph
                .forget_ticket(ticket, self.node.take().unwrap());
        }
    }
}

#[derive(Debug)]
pub struct AddModelCommand {
    model: Handle<Node>,
    sub_graph: Option<SubGraph>,
}

impl AddModelCommand {
    pub fn new(sub_graph: SubGraph) -> Self {
        Self {
            model: Default::default(),
            sub_graph: Some(sub_graph),
        }
    }
}

impl Command for AddModelCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Load Model".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        // A model was loaded, but change was reverted and here we must put all nodes
        // back to graph.
        self.model = context
            .scene
            .graph
            .put_sub_graph_back(self.sub_graph.take().unwrap());
    }

    fn revert(&mut self, context: &mut SceneContext) {
        self.sub_graph = Some(context.scene.graph.take_reserve_sub_graph(self.model));
    }

    fn finalize(&mut self, context: &mut SceneContext) {
        if let Some(sub_graph) = self.sub_graph.take() {
            context.scene.graph.forget_sub_graph(sub_graph)
        }
    }
}

#[derive(Debug)]
pub struct DeleteSubGraphCommand {
    sub_graph_root: Handle<Node>,
    sub_graph: Option<SubGraph>,
    parent: Handle<Node>,
}

impl DeleteSubGraphCommand {
    pub fn new(sub_graph_root: Handle<Node>) -> Self {
        Self {
            sub_graph_root,
            sub_graph: None,
            parent: Handle::NONE,
        }
    }
}

impl Command for DeleteSubGraphCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Delete Sub Graph".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        self.parent = context.scene.graph[self.sub_graph_root].parent();
        self.sub_graph = Some(
            context
                .scene
                .graph
                .take_reserve_sub_graph(self.sub_graph_root),
        );
    }

    fn revert(&mut self, context: &mut SceneContext) {
        context
            .scene
            .graph
            .put_sub_graph_back(self.sub_graph.take().unwrap());
        context
            .scene
            .graph
            .link_nodes(self.sub_graph_root, self.parent);
    }

    fn finalize(&mut self, context: &mut SceneContext) {
        if let Some(sub_graph) = self.sub_graph.take() {
            context.scene.graph.forget_sub_graph(sub_graph)
        }
    }
}

#[derive(Debug)]
pub struct AddNodeCommand {
    ticket: Option<Ticket<Node>>,
    handle: Handle<Node>,
    node: Option<Node>,
    cached_name: String,
    parent: Handle<Node>,
}

impl AddNodeCommand {
    pub fn new(node: Node, parent: Handle<Node>) -> Self {
        Self {
            ticket: None,
            handle: Default::default(),
            cached_name: format!("Add Node {}", node.name()),
            node: Some(node),
            parent,
        }
    }
}

impl Command for AddNodeCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        self.cached_name.clone()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        match self.ticket.take() {
            None => {
                self.handle = context.scene.graph.add_node(self.node.take().unwrap());
            }
            Some(ticket) => {
                let handle = context
                    .scene
                    .graph
                    .put_back(ticket, self.node.take().unwrap());
                assert_eq!(handle, self.handle);
            }
        }

        context.scene.graph.link_nodes(self.handle, self.parent)
    }

    fn revert(&mut self, context: &mut SceneContext) {
        // No need to unlink node from its parent because .take_reserve() does that for us.
        let (ticket, node) = context.scene.graph.take_reserve(self.handle);
        self.ticket = Some(ticket);
        self.node = Some(node);
    }

    fn finalize(&mut self, context: &mut SceneContext) {
        if let Some(ticket) = self.ticket.take() {
            context
                .scene
                .graph
                .forget_ticket(ticket, self.node.take().unwrap());
        }
    }
}

#[derive(Debug)]
pub struct ReplaceNodeCommand {
    pub handle: Handle<Node>,
    pub node: Node,
}

impl ReplaceNodeCommand {
    fn swap(&mut self, context: &mut SceneContext) {
        let existing = &mut context.scene.graph[self.handle];

        // Swap `Base` part, this is needed because base part contains hierarchy info.
        // This way base part will be moved to replacement node.
        let existing_base: &mut Base = existing;
        let replacement_base: &mut Base = &mut self.node;

        std::mem::swap(existing_base, replacement_base);

        // Now swap them completely.
        std::mem::swap(existing, &mut self.node);
    }
}

impl Command for ReplaceNodeCommand {
    fn name(&mut self, _context: &SceneContext) -> String {
        "Replace Node".to_owned()
    }

    fn execute(&mut self, context: &mut SceneContext) {
        self.swap(context);
    }

    fn revert(&mut self, context: &mut SceneContext) {
        self.swap(context);
    }
}
