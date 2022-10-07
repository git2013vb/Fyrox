use crate::{
    absm::{
        command::parameter::make_set_parameters_property_command, message::MessageSender,
        AbsmDataModel,
    },
    inspector::editors::make_property_editors_container,
    Message, MessageDirection, MSG_SYNC_FLAG,
};
use fyrox::{
    animation::machine::parameter::{Parameter, ParameterDefinition},
    core::pool::Handle,
    gui::{
        inspector::{
            editors::{
                collection::VecCollectionPropertyEditorDefinition,
                enumeration::EnumPropertyEditorDefinition,
                inspectable::InspectablePropertyEditorDefinition,
                PropertyEditorDefinitionContainer,
            },
            InspectorBuilder, InspectorContext, InspectorMessage,
        },
        message::UiMessage,
        scroll_viewer::ScrollViewerBuilder,
        widget::WidgetBuilder,
        window::{WindowBuilder, WindowTitle},
        BuildContext, UiNode, UserInterface,
    },
    utils::log::Log,
};
use std::{rc::Rc, sync::mpsc::Sender};

pub struct ParameterPanel {
    pub window: Handle<UiNode>,
    inspector: Handle<UiNode>,
    property_editors: Rc<PropertyEditorDefinitionContainer>,
}

impl ParameterPanel {
    pub fn new(ctx: &mut BuildContext, sender: Sender<Message>) -> Self {
        let property_editors = make_property_editors_container(sender);
        property_editors
            .insert(VecCollectionPropertyEditorDefinition::<ParameterDefinition>::new());
        property_editors.insert(InspectablePropertyEditorDefinition::<ParameterDefinition>::new());
        property_editors.insert(EnumPropertyEditorDefinition::<Parameter>::new());

        let inspector;
        let window = WindowBuilder::new(WidgetBuilder::new())
            .with_title(WindowTitle::text("Parameters"))
            .with_content(
                ScrollViewerBuilder::new(WidgetBuilder::new())
                    .with_content({
                        inspector = InspectorBuilder::new(WidgetBuilder::new()).build(ctx);
                        inspector
                    })
                    .build(ctx),
            )
            .build(ctx);

        Self {
            window,
            inspector,
            property_editors: Rc::new(property_editors),
        }
    }

    pub fn reset(&mut self, ui: &mut UserInterface, data_model: Option<&AbsmDataModel>) {
        let inspector_context = data_model
            .map(|data_model| {
                InspectorContext::from_object(
                    &data_model.resource.data_ref().absm_definition.parameters,
                    &mut ui.build_ctx(),
                    self.property_editors.clone(),
                    None,
                    MSG_SYNC_FLAG,
                    0,
                )
            })
            .unwrap_or_default();

        ui.send_message(InspectorMessage::context(
            self.inspector,
            MessageDirection::ToWidget,
            inspector_context,
        ));
    }

    pub fn sync_to_model(&mut self, ui: &mut UserInterface, data_model: &AbsmDataModel) {
        let ctx = ui
            .node(self.inspector)
            .cast::<fyrox::gui::inspector::Inspector>()
            .unwrap()
            .context()
            .clone();

        if let Err(sync_errors) = ctx.sync(
            &data_model.resource.data_ref().absm_definition.parameters,
            ui,
            0,
        ) {
            for error in sync_errors {
                Log::err(format!("Failed to sync property. Reason: {:?}", error))
            }
        }
    }

    pub fn handle_ui_message(&mut self, message: &UiMessage, sender: &MessageSender) {
        if message.destination() == self.inspector
            && message.direction() == MessageDirection::FromWidget
        {
            if let Some(InspectorMessage::PropertyChanged(args)) =
                message.data::<InspectorMessage>()
            {
                sender.do_command_value(make_set_parameters_property_command((), args).unwrap());
            }
        }
    }
}
