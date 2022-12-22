use crate::{
    absm::{
        canvas::{AbsmCanvasMessage, Mode},
        command::{
            AddStateCommand, DeleteStateCommand, DeleteTransitionCommand,
            SetMachineEntryStateCommand,
        },
        node::AbsmNode,
        selection::SelectedEntity,
        transition::TransitionView,
    },
    menu::create_menu_item,
    scene::{
        commands::{ChangeSelectionCommand, CommandGroup, SceneCommand},
        EditorScene, Selection,
    },
    Message,
};
use fyrox::{
    animation::machine::State,
    core::pool::Handle,
    gui::{
        menu::MenuItemMessage,
        message::{MessageDirection, UiMessage},
        popup::{Placement, PopupBuilder, PopupMessage},
        stack_panel::StackPanelBuilder,
        widget::WidgetBuilder,
        BuildContext, UiNode, UserInterface,
    },
    scene::{animation::absm::AnimationBlendingStateMachine, node::Node},
};
use std::sync::mpsc::Sender;

pub struct CanvasContextMenu {
    create_state: Handle<UiNode>,
    pub menu: Handle<UiNode>,
    pub canvas: Handle<UiNode>,
    pub node_context_menu: Handle<UiNode>,
}

impl CanvasContextMenu {
    pub fn new(ctx: &mut BuildContext) -> Self {
        let create_state;
        let menu = PopupBuilder::new(WidgetBuilder::new().with_visibility(false))
            .with_content(
                StackPanelBuilder::new(WidgetBuilder::new().with_child({
                    create_state = create_menu_item("Create State", vec![], ctx);
                    create_state
                }))
                .build(ctx),
            )
            .build(ctx);

        Self {
            create_state,
            menu,
            canvas: Default::default(),
            node_context_menu: Default::default(),
        }
    }

    pub fn handle_ui_message(
        &mut self,
        sender: &Sender<Message>,
        message: &UiMessage,
        ui: &mut UserInterface,
        absm_node_handle: Handle<Node>,
        layer_index: usize,
    ) {
        if let Some(MenuItemMessage::Click) = message.data() {
            if message.destination() == self.create_state {
                let screen_position = ui.node(self.menu).screen_position();

                sender
                    .send(Message::do_scene_command(AddStateCommand::new(
                        absm_node_handle,
                        layer_index,
                        State {
                            position: ui.node(self.canvas).screen_to_local(screen_position),
                            name: "New State".to_string(),
                            root: Default::default(),
                        },
                    )))
                    .unwrap();
            }
        }
    }
}

pub struct NodeContextMenu {
    create_transition: Handle<UiNode>,
    remove: Handle<UiNode>,
    set_as_entry_state: Handle<UiNode>,
    pub menu: Handle<UiNode>,
    pub canvas: Handle<UiNode>,
    placement_target: Handle<UiNode>,
}

impl NodeContextMenu {
    pub fn new(ctx: &mut BuildContext) -> Self {
        let create_transition;
        let remove;
        let set_as_entry_state;
        let menu = PopupBuilder::new(WidgetBuilder::new().with_visibility(false))
            .with_content(
                StackPanelBuilder::new(
                    WidgetBuilder::new()
                        .with_child({
                            create_transition = create_menu_item("Create Transition", vec![], ctx);
                            create_transition
                        })
                        .with_child({
                            remove = create_menu_item("Remove", vec![], ctx);
                            remove
                        })
                        .with_child({
                            set_as_entry_state =
                                create_menu_item("Set As Entry State", vec![], ctx);
                            set_as_entry_state
                        }),
                )
                .build(ctx),
            )
            .build(ctx);

        Self {
            create_transition,
            menu,
            remove,
            canvas: Default::default(),
            placement_target: Default::default(),
            set_as_entry_state,
        }
    }

    pub fn handle_ui_message(
        &mut self,
        message: &UiMessage,
        ui: &mut UserInterface,
        sender: &Sender<Message>,
        absm_node_handle: Handle<Node>,
        absm_node: &AnimationBlendingStateMachine,
        layer_index: usize,
        editor_scene: &EditorScene,
    ) {
        let machine = absm_node.machine();
        if let Some(MenuItemMessage::Click) = message.data() {
            if message.destination() == self.create_transition {
                ui.send_message(AbsmCanvasMessage::switch_mode(
                    self.canvas,
                    MessageDirection::ToWidget,
                    Mode::CreateTransition {
                        source: self.placement_target,
                        source_pos: ui.node(self.placement_target).center(),
                        dest_pos: ui.node(self.canvas).screen_to_local(ui.cursor_position()),
                    },
                ))
            } else if message.destination == self.remove {
                if let Selection::Absm(ref selection) = editor_scene.selection {
                    let states_to_remove = selection
                        .entities
                        .iter()
                        .cloned()
                        .filter_map(|e| {
                            if let SelectedEntity::State(handle) = e {
                                Some(handle)
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>();

                    // Gather every transition that leads from/to any of states to remove.
                    let transitions_to_remove = machine.layers()[layer_index]
                        .transitions()
                        .pair_iter()
                        .filter_map(|(handle, transition)| {
                            if states_to_remove.iter().cloned().any(|state_to_remove| {
                                state_to_remove == transition.source()
                                    || state_to_remove == transition.dest()
                            }) {
                                Some(handle)
                            } else {
                                None
                            }
                        });

                    let mut new_selection = selection.clone();
                    new_selection.entities.clear();

                    let mut group = vec![SceneCommand::new(ChangeSelectionCommand::new(
                        Selection::Absm(new_selection),
                        editor_scene.selection.clone(),
                    ))];

                    group.extend(transitions_to_remove.map(|transition| {
                        SceneCommand::new(DeleteTransitionCommand::new(
                            absm_node_handle,
                            layer_index,
                            transition,
                        ))
                    }));

                    group.extend(states_to_remove.into_iter().map(|state| {
                        SceneCommand::new(DeleteStateCommand::new(
                            absm_node_handle,
                            layer_index,
                            state,
                        ))
                    }));

                    sender
                        .send(Message::do_scene_command(CommandGroup::from(group)))
                        .unwrap();
                }
            } else if message.destination() == self.set_as_entry_state {
                sender
                    .send(Message::do_scene_command(SetMachineEntryStateCommand {
                        node_handle: absm_node_handle,
                        layer: layer_index,
                        entry: ui
                            .node(self.placement_target)
                            .query_component::<AbsmNode<State>>()
                            .unwrap()
                            .model_handle,
                    }))
                    .unwrap();
            }
        } else if let Some(PopupMessage::Placement(Placement::Cursor(target))) = message.data() {
            if message.destination() == self.menu {
                self.placement_target = *target;
            }
        }
    }
}

pub struct TransitionContextMenu {
    remove: Handle<UiNode>,
    pub menu: Handle<UiNode>,
    placement_target: Handle<UiNode>,
}

impl TransitionContextMenu {
    pub fn new(ctx: &mut BuildContext) -> Self {
        let remove;
        let menu = PopupBuilder::new(WidgetBuilder::new().with_visibility(false))
            .with_content(
                StackPanelBuilder::new(WidgetBuilder::new().with_child({
                    remove = create_menu_item("Remove Transition", vec![], ctx);
                    remove
                }))
                .build(ctx),
            )
            .build(ctx);

        Self {
            menu,
            remove,
            placement_target: Default::default(),
        }
    }

    pub fn handle_ui_message(
        &mut self,
        message: &UiMessage,
        ui: &mut UserInterface,
        sender: &Sender<Message>,
        absm_node_handle: Handle<Node>,
        layer_index: usize,
        editor_scene: &EditorScene,
    ) {
        if let Some(MenuItemMessage::Click) = message.data() {
            if message.destination == self.remove {
                if let Selection::Absm(ref selection) = editor_scene.selection {
                    let mut new_selection = selection.clone();
                    new_selection.entities.clear();

                    let transition_ref = ui
                        .node(self.placement_target)
                        .query_component::<TransitionView>()
                        .unwrap();

                    let group = vec![
                        SceneCommand::new(ChangeSelectionCommand::new(
                            Selection::Absm(new_selection),
                            editor_scene.selection.clone(),
                        )),
                        SceneCommand::new(DeleteTransitionCommand::new(
                            absm_node_handle,
                            layer_index,
                            transition_ref.model_handle,
                        )),
                    ];

                    sender
                        .send(Message::do_scene_command(CommandGroup::from(group)))
                        .unwrap();
                }
            }
        } else if let Some(PopupMessage::Placement(Placement::Cursor(target))) = message.data() {
            if message.destination() == self.menu {
                self.placement_target = *target;
            }
        }
    }
}
