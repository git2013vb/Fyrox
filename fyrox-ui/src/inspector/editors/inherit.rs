//! Property editor for [`InheritableVariable`]. It acts like a proxy to inner property, but also
//! adds special "revert" button that is used to revert value to its parent's value.

use crate::{
    button::{ButtonBuilder, ButtonMessage},
    core::{pool::Handle, reflect::prelude::*, variable::InheritableVariable},
    define_constructor,
    grid::{Column, GridBuilder, Row},
    inspector::{
        editors::{
            PropertyEditorBuildContext, PropertyEditorDefinition, PropertyEditorInstance,
            PropertyEditorMessageContext, PropertyEditorTranslationContext,
        },
        InspectorError, PropertyChanged,
    },
    inspector::{FieldKind, InheritableAction},
    message::UiMessage,
    utils::make_simple_tooltip,
    widget::WidgetBuilder,
    BuildContext, Control, MessageDirection, Thickness, UiNode, UserInterface, VerticalAlignment,
    Widget, WidgetMessage,
};
use fyrox_core::reflect::FieldValue;
use std::{
    any::{Any, TypeId},
    fmt::{Debug, Formatter},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InheritablePropertyEditorMessage {
    Revert,
    Modified(bool),
}

impl InheritablePropertyEditorMessage {
    define_constructor!(InheritablePropertyEditorMessage:Revert => fn revert(), layout: false);
    define_constructor!(InheritablePropertyEditorMessage:Modified => fn modified(bool), layout: false);
}

#[derive(Debug, Clone)]
pub struct InheritablePropertyEditor {
    widget: Widget,
    revert: Handle<UiNode>,
    inner_editor: Handle<UiNode>,
}

impl Deref for InheritablePropertyEditor {
    type Target = Widget;

    fn deref(&self) -> &Self::Target {
        &self.widget
    }
}

impl DerefMut for InheritablePropertyEditor {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.widget
    }
}

impl Control for InheritablePropertyEditor {
    fn query_component(&self, type_id: TypeId) -> Option<&dyn Any> {
        if type_id == TypeId::of::<Self>() {
            Some(self)
        } else {
            None
        }
    }

    fn handle_routed_message(&mut self, ui: &mut UserInterface, message: &mut UiMessage) {
        self.widget.handle_routed_message(ui, message);

        if let Some(ButtonMessage::Click) = message.data() {
            if message.destination() == self.revert {
                ui.send_message(InheritablePropertyEditorMessage::revert(
                    self.handle,
                    MessageDirection::FromWidget,
                ));
            }
        } else if let Some(InheritablePropertyEditorMessage::Modified(modified)) = message.data() {
            if message.destination() == self.handle {
                ui.send_message(WidgetMessage::visibility(
                    self.revert,
                    MessageDirection::ToWidget,
                    *modified,
                ));
            }
        }

        // Re-cast messages from inner editor as message from this editor.
        if message.destination() == self.inner_editor {
            let mut clone = message.clone();
            clone.destination = self.handle;
            ui.send_message(clone);
        }
    }
}

struct InheritablePropertyEditorBuilder {
    widget_builder: WidgetBuilder,
    inner_editor: Handle<UiNode>,
    container: Handle<UiNode>,
    modified: bool,
}

impl InheritablePropertyEditorBuilder {
    pub fn new(widget_builder: WidgetBuilder) -> Self {
        Self {
            widget_builder,
            inner_editor: Handle::NONE,
            container: Handle::NONE,
            modified: false,
        }
    }

    pub fn with_inner_editor(mut self, inner_editor: Handle<UiNode>) -> Self {
        self.inner_editor = inner_editor;
        self
    }

    pub fn with_container(mut self, container: Handle<UiNode>) -> Self {
        self.container = container;
        self
    }

    pub fn with_modified(mut self, modified: bool) -> Self {
        self.modified = modified;
        self
    }

    pub fn build(self, ctx: &mut BuildContext) -> Handle<UiNode> {
        let revert;
        let grid = GridBuilder::new(WidgetBuilder::new().with_child(self.container).with_child({
            revert = ButtonBuilder::new(
                WidgetBuilder::new()
                    .with_visibility(self.modified)
                    .with_width(16.0)
                    .with_height(16.0)
                    .with_vertical_alignment(VerticalAlignment::Top)
                    .with_tooltip(make_simple_tooltip(ctx, "Revert To Parent"))
                    .with_margin(Thickness::uniform(1.0))
                    .on_column(1),
            )
            .with_text("<")
            .build(ctx);
            revert
        }))
        .add_row(Row::auto())
        .add_column(Column::stretch())
        .add_column(Column::auto())
        .build(ctx);

        ctx.add_node(UiNode::new(InheritablePropertyEditor {
            widget: self.widget_builder.with_child(grid).build(),
            revert,
            inner_editor: self.inner_editor,
        }))
    }
}

pub struct InheritablePropertyEditorDefinition<T>
where
    T: FieldValue,
{
    phantom: PhantomData<T>,
}

impl<T> InheritablePropertyEditorDefinition<T>
where
    T: FieldValue,
{
    pub fn new() -> Self {
        Self {
            phantom: PhantomData,
        }
    }
}

impl<T> Debug for InheritablePropertyEditorDefinition<T>
where
    T: FieldValue,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "InheritablePropertyEditorDefinition")
    }
}

fn make_proxy<'a, 'b, T>(property_info: &'b FieldInfo<'a>) -> Result<FieldInfo<'a>, InspectorError>
where
    T: FieldValue,
    'b: 'a,
{
    let value = property_info.cast_value::<InheritableVariable<T>>()?;

    Ok(FieldInfo {
        owner_type_id: TypeId::of::<T>(),
        name: property_info.name,
        display_name: property_info.display_name,
        value: &**value,
        read_only: property_info.read_only,
        min_value: property_info.min_value,
        max_value: property_info.max_value,
        step: property_info.step,
        precision: property_info.precision,
        description: property_info.description,
        type_name: property_info.type_name,
    })
}

impl<T> PropertyEditorDefinition for InheritablePropertyEditorDefinition<T>
where
    T: FieldValue,
{
    fn value_type_id(&self) -> TypeId {
        TypeId::of::<InheritableVariable<T>>()
    }

    fn create_instance(
        &self,
        ctx: PropertyEditorBuildContext,
    ) -> Result<PropertyEditorInstance, InspectorError> {
        if let Some(definition) = ctx
            .definition_container
            .definitions()
            .get(&TypeId::of::<T>())
        {
            let instance = definition.create_instance(PropertyEditorBuildContext {
                build_context: ctx.build_context,
                property_info: &make_proxy::<T>(ctx.property_info)?,
                environment: ctx.environment.clone(),
                definition_container: ctx.definition_container.clone(),
                sync_flag: ctx.sync_flag,
                layer_index: ctx.layer_index,
            })?;

            let wrapper = InheritablePropertyEditorBuilder::new(WidgetBuilder::new())
                .with_container(match instance {
                    PropertyEditorInstance::Simple { editor } => editor,
                    PropertyEditorInstance::Custom { container, .. } => container,
                })
                .with_inner_editor(match instance {
                    PropertyEditorInstance::Simple { editor } => editor,
                    PropertyEditorInstance::Custom { editor, .. } => editor,
                })
                .with_modified(
                    ctx.property_info
                        .cast_value::<InheritableVariable<T>>()?
                        .is_modified(),
                )
                .build(ctx.build_context);

            Ok(match instance {
                PropertyEditorInstance::Simple { .. } => {
                    PropertyEditorInstance::Simple { editor: wrapper }
                }
                PropertyEditorInstance::Custom { .. } => PropertyEditorInstance::Custom {
                    container: wrapper,
                    editor: wrapper,
                },
            })
        } else {
            Err(InspectorError::Custom("No editor!".to_string()))
        }
    }

    fn create_message(
        &self,
        ctx: PropertyEditorMessageContext,
    ) -> Result<Option<UiMessage>, InspectorError> {
        if let Some(definition) = ctx
            .definition_container
            .definitions()
            .get(&TypeId::of::<T>())
        {
            let instance = ctx
                .ui
                .node(ctx.instance)
                .cast::<InheritablePropertyEditor>()
                .unwrap();

            ctx.ui
                .send_message(InheritablePropertyEditorMessage::modified(
                    instance.handle,
                    MessageDirection::ToWidget,
                    ctx.property_info
                        .cast_value::<InheritableVariable<T>>()?
                        .is_modified(),
                ));

            return definition.create_message(PropertyEditorMessageContext {
                property_info: &make_proxy::<T>(ctx.property_info)?,
                environment: ctx.environment.clone(),
                definition_container: ctx.definition_container.clone(),
                sync_flag: ctx.sync_flag,
                instance: instance.inner_editor,
                layer_index: ctx.layer_index,
                ui: ctx.ui,
            });
        }

        Err(InspectorError::Custom("No editor!".to_string()))
    }

    fn translate_message(&self, ctx: PropertyEditorTranslationContext) -> Option<PropertyChanged> {
        if let Some(InheritablePropertyEditorMessage::Revert) = ctx.message.data() {
            return Some(PropertyChanged {
                name: ctx.name.to_string(),
                owner_type_id: ctx.owner_type_id,
                value: FieldKind::Inheritable(InheritableAction::Revert),
            });
        }

        // Try translate other messages using inner property editor.
        if let Some(definition) = ctx
            .definition_container
            .definitions()
            .get(&TypeId::of::<T>())
        {
            return definition.translate_message(PropertyEditorTranslationContext {
                environment: ctx.environment.clone(),
                name: ctx.name,
                owner_type_id: ctx.owner_type_id,
                message: ctx.message,
                definition_container: ctx.definition_container.clone(),
            });
        }

        None
    }
}
