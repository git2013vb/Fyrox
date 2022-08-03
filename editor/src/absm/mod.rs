use crate::{
    absm::{
        command::{
            blend::{AddInputCommand, AddPoseSourceCommand},
            AbsmCommand, AbsmCommandStack, AbsmEditorContext,
        },
        inspector::Inspector,
        menu::Menu,
        message::{AbsmMessage, MessageSender},
        node::{AbsmNode, AbsmNodeMessage},
        parameter::ParameterPanel,
        preview::Previewer,
        state_graph::StateGraphViewer,
        state_viewer::StateViewer,
    },
    utils::{create_file_selector, open_file_selector},
    Message,
};
use fyrox::{
    animation::machine::{
        node::{
            blend::{BlendPoseDefinition, IndexedBlendInputDefinition},
            PoseNodeDefinition,
        },
        state::StateDefinition,
        transition::TransitionDefinition,
        Event, MachineDefinition,
    },
    asset::{Resource, ResourceState},
    core::{
        color::Color,
        futures::executor::block_on,
        pool::Handle,
        visitor::{Visit, VisitResult, Visitor},
    },
    engine::Engine,
    gui::{
        dock::{DockingManagerBuilder, TileBuilder, TileContent},
        file_browser::{FileBrowserMode, FileSelectorMessage},
        grid::{Column, GridBuilder, Row},
        message::{MessageDirection, UiMessage},
        widget::WidgetBuilder,
        window::{WindowBuilder, WindowMessage, WindowTitle},
        UiNode, UserInterface,
    },
    resource::absm::{AbsmResource, AbsmResourceState},
    utils::log::Log,
};
use std::{
    path::{Path, PathBuf},
    sync::mpsc::{channel, Receiver, Sender},
};

mod canvas;
mod command;
mod connection;
mod inspector;
mod menu;
mod message;
mod node;
mod parameter;
mod preview;
mod segment;
mod selectable;
mod socket;
mod state_graph;
mod state_viewer;
mod transition;

const NORMAL_BACKGROUND: Color = Color::opaque(60, 60, 60);
const SELECTED_BACKGROUND: Color = Color::opaque(80, 80, 80);
const BORDER_COLOR: Color = Color::opaque(70, 70, 70);
const NORMAL_ROOT_COLOR: Color = Color::opaque(40, 80, 0);
const SELECTED_ROOT_COLOR: Color = Color::opaque(60, 100, 0);

#[derive(PartialEq, Eq, Debug, Clone)]
pub enum SelectedEntity {
    Transition(Handle<TransitionDefinition>),
    State(Handle<StateDefinition>),
    PoseNode(Handle<PoseNodeDefinition>),
}

pub struct AbsmDataModel {
    path: PathBuf,
    preview_model_path: PathBuf,
    selection: Vec<SelectedEntity>,
    resource: AbsmResource,
}

impl AbsmDataModel {
    pub fn new() -> Self {
        Self {
            path: Default::default(),
            preview_model_path: Default::default(),
            selection: Default::default(),
            resource: AbsmResource::from(Resource::new(ResourceState::Ok(AbsmResourceState {
                path: Default::default(),
                absm_definition: Default::default(),
            }))),
        }
    }

    pub fn ctx(&mut self) -> AbsmEditorContext {
        AbsmEditorContext {
            selection: &mut self.selection,
            resource: self.resource.data_ref(),
        }
    }

    // Manual implementation is needed to store editor data alongside the engine data.
    fn visit(&mut self, visitor: &mut Visitor) -> VisitResult {
        // Visit engine data first.
        if visitor.is_reading() {
            let mut definition = MachineDefinition::default();
            definition.visit("Machine", visitor)?;

            self.resource =
                AbsmResource::from(Resource::new(ResourceState::Ok(AbsmResourceState {
                    path: Default::default(),
                    absm_definition: definition,
                })));
        } else {
            self.resource
                .data_ref()
                .absm_definition
                .visit("Machine", visitor)?;
        }

        // Visit editor-specific data. These fields are optional so we ignore any errors here.
        let _ = self.preview_model_path.visit("PreviewModelPath", visitor);

        Ok(())
    }
}

pub struct AbsmEditor {
    window: Handle<UiNode>,
    command_stack: AbsmCommandStack,
    data_model: Option<AbsmDataModel>,
    message_sender: MessageSender,
    message_receiver: Receiver<AbsmMessage>,
    inspector: Inspector,
    state_graph_viewer: StateGraphViewer,
    save_dialog: Handle<UiNode>,
    load_dialog: Handle<UiNode>,
    previewer: Previewer,
    state_viewer: StateViewer,
    menu: Menu,
    parameter_panel: ParameterPanel,
}

impl AbsmEditor {
    pub fn new(engine: &mut Engine, sender: Sender<Message>) -> Self {
        let (tx, rx) = channel();

        let previewer = Previewer::new(engine);

        let ui = &mut engine.user_interface;
        let ctx = &mut ui.build_ctx();

        let menu = Menu::new(ctx);

        let inspector = Inspector::new(ctx, sender.clone());
        let state_graph_viewer = StateGraphViewer::new(ctx);
        let state_viewer = StateViewer::new(ctx);
        let parameter_panel = ParameterPanel::new(ctx, sender);

        let docking_manager = DockingManagerBuilder::new(
            WidgetBuilder::new().on_row(1).with_child(
                TileBuilder::new(WidgetBuilder::new())
                    .with_content(TileContent::HorizontalTiles {
                        splitter: 0.8,
                        tiles: [
                            TileBuilder::new(WidgetBuilder::new())
                                .with_content(TileContent::HorizontalTiles {
                                    splitter: 0.3,
                                    tiles: [
                                        TileBuilder::new(WidgetBuilder::new())
                                            .with_content(TileContent::VerticalTiles {
                                                splitter: 0.5,
                                                tiles: [
                                                    TileBuilder::new(WidgetBuilder::new())
                                                        .with_content(TileContent::Window(
                                                            previewer.window,
                                                        ))
                                                        .build(ctx),
                                                    TileBuilder::new(WidgetBuilder::new())
                                                        .with_content(TileContent::Window(
                                                            parameter_panel.window,
                                                        ))
                                                        .build(ctx),
                                                ],
                                            })
                                            .build(ctx),
                                        TileBuilder::new(WidgetBuilder::new())
                                            .with_content(TileContent::HorizontalTiles {
                                                splitter: 0.5,
                                                tiles: [
                                                    TileBuilder::new(WidgetBuilder::new())
                                                        .with_content(TileContent::Window(
                                                            state_graph_viewer.window,
                                                        ))
                                                        .build(ctx),
                                                    TileBuilder::new(WidgetBuilder::new())
                                                        .with_content(TileContent::Window(
                                                            state_viewer.window,
                                                        ))
                                                        .build(ctx),
                                                ],
                                            })
                                            .build(ctx),
                                    ],
                                })
                                .build(ctx),
                            TileBuilder::new(WidgetBuilder::new())
                                .with_content(TileContent::Window(inspector.window))
                                .build(ctx),
                        ],
                    })
                    .build(ctx),
            ),
        )
        .build(ctx);

        let window = WindowBuilder::new(WidgetBuilder::new().with_width(1200.0).with_height(700.0))
            .open(false)
            .with_content(
                GridBuilder::new(
                    WidgetBuilder::new()
                        .with_child(menu.menu)
                        .with_child(docking_manager),
                )
                .add_row(Row::strict(24.0))
                .add_row(Row::stretch())
                .add_column(Column::stretch())
                .build(ctx),
            )
            .with_title(WindowTitle::text("ABSM Editor"))
            .build(ctx);

        let load_dialog = create_file_selector(ctx, "absm", FileBrowserMode::Open);
        let save_dialog = create_file_selector(
            ctx,
            "absm",
            FileBrowserMode::Save {
                default_file_name: PathBuf::from("unnamed.absm"),
            },
        );

        Self {
            window,
            message_sender: MessageSender::new(tx),
            message_receiver: rx,
            command_stack: AbsmCommandStack::new(false),
            data_model: None,
            menu,
            state_graph_viewer,
            inspector,
            save_dialog,
            load_dialog,
            previewer,
            state_viewer,
            parameter_panel,
        }
    }

    fn sync_to_model(&mut self, engine: &mut Engine) {
        if let Some(data_model) = self.data_model.as_ref() {
            let ui = &mut engine.user_interface;
            self.parameter_panel.sync_to_model(ui, data_model);
            self.state_graph_viewer.sync_to_model(data_model, ui);
            self.state_viewer.sync_to_model(ui, data_model);
            self.inspector.sync_to_model(ui, data_model);
            self.previewer.set_absm(engine, &data_model.resource);
        }
    }

    fn do_command(&mut self, command: AbsmCommand) -> bool {
        if let Some(data_model) = self.data_model.as_mut() {
            self.command_stack
                .do_command(command.into_inner(), data_model.ctx());
            true
        } else {
            false
        }
    }

    fn undo_command(&mut self) -> bool {
        if let Some(data_model) = self.data_model.as_mut() {
            self.command_stack.undo(data_model.ctx());
            true
        } else {
            false
        }
    }

    fn redo_command(&mut self) -> bool {
        if let Some(data_model) = self.data_model.as_mut() {
            self.command_stack.redo(data_model.ctx());
            true
        } else {
            false
        }
    }

    fn clear_command_stack(&mut self) -> bool {
        if let Some(data_model) = self.data_model.as_mut() {
            self.command_stack.clear(data_model.ctx());
            true
        } else {
            false
        }
    }

    fn set_data_model(&mut self, engine: &mut Engine, data_model: Option<AbsmDataModel>) {
        self.clear_command_stack();
        self.state_viewer.clear(&engine.user_interface);

        self.data_model = data_model;

        if let Some(data_model) = self.data_model.as_ref() {
            self.parameter_panel
                .reset(&mut engine.user_interface, Some(data_model));
            self.previewer.set_preview_model(
                engine,
                &data_model.preview_model_path,
                &data_model.resource,
            );
            self.sync_to_model(engine);
        } else {
            self.state_graph_viewer.clear(&engine.user_interface);
            self.previewer.clear(engine);
            self.parameter_panel.reset(&mut engine.user_interface, None);
            self.inspector.clear(&engine.user_interface);
        }
    }

    fn create_new_absm(&mut self, engine: &mut Engine) {
        self.set_data_model(engine, Some(AbsmDataModel::new()));
    }

    fn open_save_dialog(&self, ui: &UserInterface) {
        open_file_selector(self.save_dialog, ui);
    }

    fn open_load_dialog(&self, ui: &UserInterface) {
        open_file_selector(self.load_dialog, ui);
    }

    fn save_current_absm(&mut self, path: PathBuf) {
        if let Some(data_model) = self.data_model.as_mut() {
            data_model.path = path.clone();

            let mut visitor = Visitor::new();
            Log::verify(data_model.visit(&mut visitor));
            Log::verify(visitor.save_binary(path));
        }
    }

    fn set_preview_model(&mut self, engine: &mut Engine, path: &Path) {
        if let Some(data_model) = self.data_model.as_mut() {
            self.previewer
                .set_preview_model(engine, path, &data_model.resource);

            data_model.preview_model_path = path.to_path_buf();
        }
    }

    fn load_absm(&mut self, path: &Path, engine: &mut Engine) {
        match block_on(Visitor::load_binary(path)) {
            Ok(mut visitor) => {
                let mut data_model = AbsmDataModel::new();
                if let Err(e) = data_model.visit(&mut visitor) {
                    Log::err(format!(
                        "Unable to read ABSM from {}. Reason: {}",
                        path.display(),
                        e
                    ));
                } else {
                    data_model.path = path.to_path_buf();
                    self.set_data_model(engine, Some(data_model));
                }
            }
            Err(e) => Log::err(format!(
                "Unable to load ABSM from {}. Reason: {}",
                path.display(),
                e
            )),
        };
    }

    pub fn open(&self, ui: &UserInterface) {
        ui.send_message(WindowMessage::open(
            self.window,
            MessageDirection::ToWidget,
            true,
        ));
    }

    pub fn update(&mut self, engine: &mut Engine) {
        let mut need_sync = false;

        while let Ok(message) = self.message_receiver.try_recv() {
            match message {
                AbsmMessage::DoCommand(command) => {
                    need_sync |= self.do_command(command);
                }
                AbsmMessage::Undo => {
                    need_sync |= self.undo_command();
                }
                AbsmMessage::Redo => {
                    need_sync |= self.redo_command();
                }
                AbsmMessage::ClearCommandStack => {
                    need_sync |= self.clear_command_stack();
                }
                AbsmMessage::CreateNewAbsm => self.create_new_absm(engine),
                AbsmMessage::LoadAbsm => {
                    self.open_load_dialog(&engine.user_interface);
                }
                AbsmMessage::SaveCurrentAbsm => {
                    if let Some(data_model) = self.data_model.as_ref() {
                        if data_model.path.exists() {
                            let path = data_model.path.clone();
                            self.save_current_absm(path)
                        } else {
                            self.open_save_dialog(&engine.user_interface);
                        }
                    }
                }
                AbsmMessage::Sync => {
                    need_sync = true;
                }
                AbsmMessage::SetPreviewModel(path) => self.set_preview_model(engine, &path),
            }
        }

        if need_sync {
            self.sync_to_model(engine);
        }

        self.previewer.update(engine);

        self.handle_machine_events(engine);
    }

    pub fn handle_machine_events(&self, engine: &mut Engine) {
        let scene = &mut engine.scenes[self.previewer.scene()];

        if let Some(machine) = scene
            .animation_machines
            .try_get_mut(self.previewer.current_absm())
        {
            while let Some(event) = machine.pop_event() {
                match event {
                    Event::ActiveStateChanged(state) => {
                        if let Some(state_ref) = machine.states().try_borrow(state) {
                            self.state_graph_viewer
                                .activate_state(&engine.user_interface, state_ref.definition);
                        }
                    }
                    Event::ActiveTransitionChanged(transition) => {
                        if let Some(transition_ref) = machine.transitions().try_borrow(transition) {
                            self.state_graph_viewer.activate_transition(
                                &engine.user_interface,
                                transition_ref.definition,
                            );
                        }
                    }
                    _ => (),
                }
            }
        }
    }

    pub fn handle_ui_message(&mut self, message: &UiMessage, engine: &mut Engine) {
        self.previewer
            .handle_message(message, &self.message_sender, engine);

        let ui = &mut engine.user_interface;
        self.menu.handle_ui_message(&self.message_sender, message);

        if let Some(data_model) = self.data_model.as_ref() {
            self.state_viewer
                .handle_ui_message(message, ui, &self.message_sender, data_model);
            self.state_graph_viewer.handle_ui_message(
                message,
                ui,
                &self.message_sender,
                data_model,
            );
            self.inspector
                .handle_ui_message(message, data_model, &self.message_sender);
            self.parameter_panel
                .handle_ui_message(message, &self.message_sender);
        }

        if let Some(FileSelectorMessage::Commit(path)) = message.data() {
            if message.destination() == self.save_dialog {
                self.save_current_absm(path.clone())
            } else if message.destination() == self.load_dialog {
                self.load_absm(path, engine);
            }
        } else if let Some(WindowMessage::Close) = message.data() {
            if message.destination() == self.window {
                // Clear on close.
                self.set_data_model(engine, None);
            }
        } else if let Some(msg) = message.data::<AbsmNodeMessage>() {
            if let Some(data_model) = self.data_model.as_ref() {
                match msg {
                    AbsmNodeMessage::Enter => {
                        if let Some(node) = ui
                            .node(message.destination())
                            .query_component::<AbsmNode<StateDefinition>>()
                        {
                            self.state_viewer
                                .set_state(node.model_handle, data_model, ui);
                            self.message_sender.sync();
                        }
                    }
                    AbsmNodeMessage::AddInput => {
                        if let Some(node) = ui
                            .node(message.destination())
                            .query_component::<AbsmNode<PoseNodeDefinition>>()
                        {
                            let model_ref = &data_model.resource.data_ref().absm_definition.nodes
                                [node.model_handle];

                            match model_ref {
                                PoseNodeDefinition::PlayAnimation(_) => {
                                    // No input sockets
                                }
                                PoseNodeDefinition::BlendAnimations(_) => {
                                    self.message_sender.do_command(AddPoseSourceCommand::new(
                                        node.model_handle,
                                        BlendPoseDefinition::default(),
                                    ));
                                }
                                PoseNodeDefinition::BlendAnimationsByIndex(_) => {
                                    self.message_sender.do_command(AddInputCommand::new(
                                        node.model_handle,
                                        IndexedBlendInputDefinition::default(),
                                    ));
                                }
                            }
                        }
                    }
                    _ => (),
                }
            }
        }
    }
}
