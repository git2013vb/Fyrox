use crate::{
    scene::commands::{
        make_set_node_property_command,
        terrain::{AddTerrainLayerCommand, DeleteTerrainLayerCommand},
    },
    SceneCommand,
};
use fyrox::{
    core::pool::Handle,
    gui::inspector::{CollectionChanged, FieldKind, PropertyChanged},
    scene::{node::Node, terrain::Terrain},
};
use std::any::TypeId;

pub struct SceneNodePropertyChangedHandler;

impl SceneNodePropertyChangedHandler {
    fn try_get_command(
        &self,
        args: &PropertyChanged,
        handle: Handle<Node>,
        node: &mut Node,
    ) -> Option<SceneCommand> {
        // Terrain is special and have its own commands for specific properties.
        if args.path() == Terrain::LAYERS && args.owner_type_id == TypeId::of::<Terrain>() {
            match args.value {
                FieldKind::Collection(ref collection_changed) => match **collection_changed {
                    CollectionChanged::Add(_) => Some(SceneCommand::new(
                        AddTerrainLayerCommand::new(handle, node.as_terrain()),
                    )),
                    CollectionChanged::Remove(index) => Some(SceneCommand::new(
                        DeleteTerrainLayerCommand::new(handle, index),
                    )),
                    CollectionChanged::ItemChanged { .. } => None,
                },
                _ => None,
            }
        } else {
            None
        }
    }
}

impl SceneNodePropertyChangedHandler {
    pub fn handle(
        &self,
        args: &PropertyChanged,
        handle: Handle<Node>,
        node: &mut Node,
    ) -> SceneCommand {
        self.try_get_command(args, handle, node)
            .unwrap_or_else(|| make_set_node_property_command(handle, args))
    }
}
