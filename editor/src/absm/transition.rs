use fyrox::{
    core::{algebra::Vector2, math::Rect, pool::Handle},
    gui::{
        define_constructor, define_widget_deref,
        draw::{CommandTexture, Draw, DrawingContext},
        message::{MessageDirection, UiMessage},
        utils::{make_arrow_primitives, ArrowDirection},
        vector_image::VectorImageBuilder,
        widget::{Widget, WidgetBuilder},
        BuildContext, Control, UiNode, UserInterface,
    },
};
use std::{
    any::{Any, TypeId},
    ops::{Deref, DerefMut},
};

#[derive(Clone, Debug)]
pub struct Transition {
    widget: Widget,
    pub source: Handle<UiNode>,
    source_pos: Vector2<f32>,
    pub dest: Handle<UiNode>,
    dest_pos: Vector2<f32>,
    arrow: Handle<UiNode>,
}

define_widget_deref!(Transition);

#[derive(Debug, Clone, PartialEq)]
pub enum TransitionMessage {
    SourcePosition(Vector2<f32>),
    DestPosition(Vector2<f32>),
}

impl TransitionMessage {
    define_constructor!(TransitionMessage:SourcePosition => fn source_position(Vector2<f32>), layout: false);
    define_constructor!(TransitionMessage:DestPosition => fn dest_position(Vector2<f32>), layout: false);
}

impl Control for Transition {
    fn query_component(&self, type_id: TypeId) -> Option<&dyn Any> {
        if type_id == TypeId::of::<Self>() {
            Some(self)
        } else {
            None
        }
    }

    fn draw(&self, drawing_context: &mut DrawingContext) {
        drawing_context.push_line(self.source_pos, self.dest_pos, 2.0);
        drawing_context.commit(
            Rect::new(0.0, 0.0, 9999.0, 9999.0),
            self.foreground(),
            CommandTexture::None,
            None,
        );
    }

    fn handle_routed_message(&mut self, ui: &mut UserInterface, message: &mut UiMessage) {
        self.widget.handle_routed_message(ui, message);

        if let Some(msg) = message.data::<TransitionMessage>() {
            if message.destination() == self.handle()
                && message.direction() == MessageDirection::ToWidget
            {
                match msg {
                    TransitionMessage::SourcePosition(pos) => {
                        self.source_pos = *pos;
                    }
                    TransitionMessage::DestPosition(pos) => {
                        self.dest_pos = *pos;
                    }
                }
            }
        }
    }
}

pub struct TransitionBuilder {
    widget_builder: WidgetBuilder,
    source: Handle<UiNode>,
    dest: Handle<UiNode>,
}

impl TransitionBuilder {
    pub fn new(widget_builder: WidgetBuilder) -> Self {
        Self {
            widget_builder,
            source: Default::default(),
            dest: Default::default(),
        }
    }

    pub fn with_source(mut self, source: Handle<UiNode>) -> Self {
        self.source = source;
        self
    }

    pub fn with_dest(mut self, dest: Handle<UiNode>) -> Self {
        self.dest = dest;
        self
    }

    pub fn build(self, ctx: &mut BuildContext) -> Handle<UiNode> {
        let arrow = VectorImageBuilder::new(WidgetBuilder::new())
            .with_primitives(make_arrow_primitives(ArrowDirection::Right, 10.0))
            .build(ctx);

        let transition = Transition {
            widget: self.widget_builder.build(),
            source: self.source,
            source_pos: Default::default(),
            dest: self.dest,
            dest_pos: Default::default(),
            arrow,
        };

        ctx.add_node(UiNode::new(transition))
    }
}