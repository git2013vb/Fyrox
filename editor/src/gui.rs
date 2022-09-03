use fyrox::{
    core::{algebra::Vector2, color::Color, pool::Handle},
    gui::{
        border::BorderBuilder,
        brush::Brush,
        button::ButtonBuilder,
        decorator::DecoratorBuilder,
        define_constructor,
        draw::SharedTexture,
        formatted_text::WrapMode,
        image::ImageBuilder,
        message::{MessageDirection, UiMessage},
        text::TextBuilder,
        widget::WidgetBuilder,
        BuildContext, HorizontalAlignment, Thickness, UiNode, VerticalAlignment,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetItemMessage {
    Select(bool),
}

pub fn make_dropdown_list_option(ctx: &mut BuildContext, name: &str) -> Handle<UiNode> {
    DecoratorBuilder::new(BorderBuilder::new(
        WidgetBuilder::new().with_child(
            TextBuilder::new(WidgetBuilder::new())
                .with_vertical_text_alignment(VerticalAlignment::Center)
                .with_horizontal_text_alignment(HorizontalAlignment::Center)
                .with_text(name)
                .build(ctx),
        ),
    ))
    .build(ctx)
}

pub fn make_dropdown_list_option_with_height(
    ctx: &mut BuildContext,
    name: &str,
    height: f32,
) -> Handle<UiNode> {
    DecoratorBuilder::new(BorderBuilder::new(
        WidgetBuilder::new().with_height(height).with_child(
            TextBuilder::new(WidgetBuilder::new())
                .with_vertical_text_alignment(VerticalAlignment::Center)
                .with_horizontal_text_alignment(HorizontalAlignment::Center)
                .with_text(name)
                .build(ctx),
        ),
    ))
    .build(ctx)
}

impl AssetItemMessage {
    define_constructor!(AssetItemMessage:Select => fn select(bool), layout: false);
}

pub fn make_image_button_with_tooltip(
    ctx: &mut BuildContext,
    width: f32,
    height: f32,
    image: Option<SharedTexture>,
    tooltip: &str,
) -> Handle<UiNode> {
    ButtonBuilder::new(
        WidgetBuilder::new()
            .with_tooltip(
                BorderBuilder::new(
                    WidgetBuilder::new()
                        .with_max_size(Vector2::new(300.0, f32::MAX))
                        .with_visibility(false)
                        .with_child(
                            TextBuilder::new(
                                WidgetBuilder::new().with_margin(Thickness::uniform(2.0)),
                            )
                            .with_wrap(WrapMode::Word)
                            .with_text(tooltip)
                            .build(ctx),
                        ),
                )
                .build(ctx),
            )
            .with_margin(Thickness::uniform(1.0)),
    )
    .with_content(
        ImageBuilder::new(
            WidgetBuilder::new()
                .with_background(Brush::Solid(Color::opaque(180, 180, 180)))
                .with_margin(Thickness::uniform(1.0))
                .with_width(width)
                .with_height(height),
        )
        .with_opt_texture(image)
        .build(ctx),
    )
    .build(ctx)
}
