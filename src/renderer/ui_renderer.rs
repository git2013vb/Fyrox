//! See [`UiRenderer`] docs.

use crate::renderer::framework::framebuffer::BlendParameters;
use crate::{
    asset::Resource,
    core::{
        algebra::{Matrix4, Vector2, Vector4},
        color::Color,
        math::Rect,
        parking_lot::Mutex,
        scope_profile,
        sstorage::ImmutableString,
    },
    gui::{
        brush::Brush,
        draw::{CommandTexture, DrawingContext, SharedTexture},
    },
    renderer::{
        framework::{
            error::FrameworkError,
            framebuffer::{DrawParameters, FrameBuffer},
            geometry_buffer::{
                AttributeDefinition, AttributeKind, BufferBuilder, ElementKind, GeometryBuffer,
                GeometryBufferBuilder, GeometryBufferKind,
            },
            gpu_program::{GpuProgram, UniformLocation},
            gpu_texture::GpuTexture,
            state::{
                BlendFactor, BlendFunc, ColorMask, CompareFunc, PipelineState, StencilAction,
                StencilFunc, StencilOp,
            },
        },
        RenderPassStatistics, TextureCache,
    },
    resource::texture::{Texture, TextureData, TextureKind, TexturePixelKind, TextureState},
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

struct UiShader {
    program: GpuProgram,
    wvp_matrix: UniformLocation,
    diffuse_texture: UniformLocation,
    is_font: UniformLocation,
    solid_color: UniformLocation,
    brush_type: UniformLocation,
    gradient_point_count: UniformLocation,
    gradient_colors: UniformLocation,
    gradient_stops: UniformLocation,
    gradient_origin: UniformLocation,
    gradient_end: UniformLocation,
    resolution: UniformLocation,
    bounds_min: UniformLocation,
    bounds_max: UniformLocation,
    opacity: UniformLocation,
}

impl UiShader {
    pub fn new(state: &mut PipelineState) -> Result<Self, FrameworkError> {
        let fragment_source = include_str!("shaders/ui_fs.glsl");
        let vertex_source = include_str!("shaders/ui_vs.glsl");
        let program = GpuProgram::from_source(state, "UIShader", vertex_source, fragment_source)?;
        Ok(Self {
            wvp_matrix: program
                .uniform_location(state, &ImmutableString::new("worldViewProjection"))?,
            diffuse_texture: program
                .uniform_location(state, &ImmutableString::new("diffuseTexture"))?,
            is_font: program.uniform_location(state, &ImmutableString::new("isFont"))?,
            solid_color: program.uniform_location(state, &ImmutableString::new("solidColor"))?,
            brush_type: program.uniform_location(state, &ImmutableString::new("brushType"))?,
            gradient_point_count: program
                .uniform_location(state, &ImmutableString::new("gradientPointCount"))?,
            gradient_colors: program
                .uniform_location(state, &ImmutableString::new("gradientColors"))?,
            gradient_stops: program
                .uniform_location(state, &ImmutableString::new("gradientStops"))?,
            gradient_origin: program
                .uniform_location(state, &ImmutableString::new("gradientOrigin"))?,
            gradient_end: program.uniform_location(state, &ImmutableString::new("gradientEnd"))?,
            bounds_min: program.uniform_location(state, &ImmutableString::new("boundsMin"))?,
            bounds_max: program.uniform_location(state, &ImmutableString::new("boundsMax"))?,
            resolution: program.uniform_location(state, &ImmutableString::new("resolution"))?,
            opacity: program.uniform_location(state, &ImmutableString::new("opacity"))?,
            program,
        })
    }
}

/// User interface renderer allows you to render drawing context in specified render target.
pub struct UiRenderer {
    shader: UiShader,
    geometry_buffer: GeometryBuffer,
    clipping_geometry_buffer: GeometryBuffer,
}

/// A set of parameters to render a specified user interface drawing context.
pub struct UiRenderContext<'a, 'b, 'c> {
    /// Render pipeline state.
    pub state: &'a mut PipelineState,
    /// Viewport to where render the user interface.
    pub viewport: Rect<i32>,
    /// Frame buffer to where render the user interface.
    pub frame_buffer: &'b mut FrameBuffer,
    /// Width of the frame buffer to where render the user interface.
    pub frame_width: f32,
    /// Height of the frame buffer to where render the user interface.
    pub frame_height: f32,
    /// Drawing context of a user interface.
    pub drawing_context: &'c DrawingContext,
    /// A reference of white-pixel texture.
    pub white_dummy: Rc<RefCell<GpuTexture>>,
    /// GPU texture cache.
    pub texture_cache: &'a mut TextureCache,
}

impl UiRenderer {
    pub(in crate::renderer) fn new(state: &mut PipelineState) -> Result<Self, FrameworkError> {
        let geometry_buffer = GeometryBufferBuilder::new(ElementKind::Triangle)
            .with_buffer_builder(
                BufferBuilder::new::<crate::gui::draw::Vertex>(
                    GeometryBufferKind::DynamicDraw,
                    None,
                )
                .with_attribute(AttributeDefinition {
                    location: 0,
                    kind: AttributeKind::Float2,
                    normalized: false,
                    divisor: 0,
                })
                .with_attribute(AttributeDefinition {
                    location: 1,
                    kind: AttributeKind::Float2,
                    normalized: false,
                    divisor: 0,
                })
                .with_attribute(AttributeDefinition {
                    location: 2,
                    kind: AttributeKind::UnsignedByte4,
                    normalized: true, // Make sure [0; 255] -> [0; 1]
                    divisor: 0,
                }),
            )
            .build(state)?;

        let clipping_geometry_buffer = GeometryBufferBuilder::new(ElementKind::Triangle)
            .with_buffer_builder(
                BufferBuilder::new::<crate::gui::draw::Vertex>(
                    GeometryBufferKind::DynamicDraw,
                    None,
                )
                // We're interested only in position. Fragment shader won't run for clipping geometry anyway.
                .with_attribute(AttributeDefinition {
                    location: 0,
                    kind: AttributeKind::Float2,
                    normalized: false,
                    divisor: 0,
                }),
            )
            .build(state)?;

        Ok(Self {
            geometry_buffer,
            clipping_geometry_buffer,
            shader: UiShader::new(state)?,
        })
    }

    /// Renders given UI's drawing context to specified frame buffer.
    pub fn render(
        &mut self,
        args: UiRenderContext,
    ) -> Result<RenderPassStatistics, FrameworkError> {
        scope_profile!();

        let UiRenderContext {
            state,
            viewport,
            frame_buffer,
            frame_width,
            frame_height,
            drawing_context,
            white_dummy,
            texture_cache,
        } = args;

        let mut statistics = RenderPassStatistics::default();

        self.geometry_buffer
            .set_buffer_data(state, 0, drawing_context.get_vertices());

        let geometry_buffer = self.geometry_buffer.bind(state);
        geometry_buffer.set_triangles(drawing_context.get_triangles());

        let ortho = Matrix4::new_orthographic(0.0, frame_width, frame_height, 0.0, -1.0, 1.0);
        let resolution = Vector2::new(frame_width, frame_height);

        state.set_scissor_test(true);

        for cmd in drawing_context.get_commands() {
            let mut diffuse_texture = white_dummy.clone();
            let mut is_font_texture = false;

            let mut clip_bounds = cmd.clip_bounds;
            clip_bounds.position.x = clip_bounds.position.x.floor();
            clip_bounds.position.y = clip_bounds.position.y.floor();
            clip_bounds.size.x = clip_bounds.size.x.ceil();
            clip_bounds.size.y = clip_bounds.size.y.ceil();

            state.set_scissor_box(
                clip_bounds.position.x as i32,
                // Because OpenGL is was designed for mathematicians, it has origin at lower left corner.
                viewport.size.y - (clip_bounds.position.y + clip_bounds.size.y) as i32,
                clip_bounds.size.x as i32,
                clip_bounds.size.y as i32,
            );

            let mut stencil_test = None;

            // Draw clipping geometry first if we have any. This is optional, because complex
            // clipping is very rare and in most cases scissor test will do the job.
            if let Some(clipping_geometry) = cmd.clipping_geometry.as_ref() {
                frame_buffer.clear(state, viewport, None, None, Some(0));

                self.clipping_geometry_buffer.set_buffer_data(
                    state,
                    0,
                    &clipping_geometry.vertex_buffer,
                );
                self.clipping_geometry_buffer
                    .bind(state)
                    .set_triangles(&clipping_geometry.triangle_buffer);

                // Draw
                statistics += frame_buffer.draw(
                    &self.clipping_geometry_buffer,
                    state,
                    viewport,
                    &self.shader.program,
                    &DrawParameters {
                        cull_face: None,
                        color_write: ColorMask::all(false),
                        depth_write: false,
                        stencil_test: None,
                        depth_test: false,
                        blend: None,
                        stencil_op: StencilOp {
                            zpass: StencilAction::Incr,
                            ..Default::default()
                        },
                    },
                    |mut program_binding| {
                        program_binding.set_matrix4(&self.shader.wvp_matrix, &ortho);
                    },
                );

                // Make sure main geometry will be drawn only on marked pixels.
                stencil_test = Some(StencilFunc {
                    func: CompareFunc::Equal,
                    ref_value: 1,
                    ..Default::default()
                });
            }

            match &cmd.texture {
                CommandTexture::Font(font_arc) => {
                    let mut font = font_arc.0.lock();
                    if font.texture.is_none() {
                        let size = font.atlas_size() as u32;
                        if let Some(details) = TextureData::from_bytes(
                            TextureKind::Rectangle {
                                width: size,
                                height: size,
                            },
                            TexturePixelKind::R8,
                            font.atlas_pixels().to_vec(),
                            false,
                        ) {
                            font.texture = Some(SharedTexture(Arc::new(Mutex::new(
                                TextureState::Ok(details),
                            ))));
                        }
                    }
                    let tex = font
                        .texture
                        .clone()
                        .unwrap()
                        .0
                        .downcast::<Mutex<TextureState>>()
                        .unwrap();
                    if let Some(texture) = texture_cache.get(state, &Texture(Resource::from(tex))) {
                        diffuse_texture = texture;
                    }
                    is_font_texture = true;
                }
                CommandTexture::Texture(texture) => {
                    if let Ok(texture) = texture.clone().0.downcast::<Mutex<TextureState>>() {
                        let resource = Resource::from(texture);
                        if let Some(texture) = texture_cache.get(state, &Texture(resource)) {
                            diffuse_texture = texture;
                        }
                    }
                }
                _ => (),
            }

            let mut raw_stops = [0.0; 16];
            let mut raw_colors = [Vector4::default(); 16];
            let bounds_max = cmd.bounds.right_bottom_corner();

            let (gradient_origin, gradient_end) = match cmd.brush {
                Brush::Solid(_) => (Vector2::default(), Vector2::default()),
                Brush::LinearGradient { from, to, .. } => (from, to),
                Brush::RadialGradient { center, .. } => (center, Vector2::default()),
            };

            let params = DrawParameters {
                cull_face: None,
                color_write: ColorMask::all(true),
                depth_write: false,
                stencil_test,
                depth_test: false,
                blend: Some(BlendParameters {
                    func: BlendFunc::new(BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                    ..Default::default()
                }),
                stencil_op: Default::default(),
            };

            let shader = &self.shader;
            statistics += frame_buffer.draw_part(
                &self.geometry_buffer,
                state,
                viewport,
                &self.shader.program,
                params,
                cmd.triangles.start,
                cmd.triangles.end - cmd.triangles.start,
                |mut program_binding| {
                    program_binding
                        .set_texture(&shader.diffuse_texture, &diffuse_texture)
                        .set_matrix4(&shader.wvp_matrix, &ortho)
                        .set_vector2(&shader.resolution, &resolution)
                        .set_vector2(&shader.bounds_min, &cmd.bounds.position)
                        .set_vector2(&shader.bounds_max, &bounds_max)
                        .set_bool(&shader.is_font, is_font_texture)
                        .set_i32(
                            &shader.brush_type,
                            match cmd.brush {
                                Brush::Solid(_) => 0,
                                Brush::LinearGradient { .. } => 1,
                                Brush::RadialGradient { .. } => 2,
                            },
                        )
                        .set_srgb_color(
                            &shader.solid_color,
                            &match cmd.brush {
                                Brush::Solid(color) => color,
                                _ => Color::WHITE,
                            },
                        )
                        .set_vector2(&shader.gradient_origin, &gradient_origin)
                        .set_vector2(&shader.gradient_end, &gradient_end)
                        .set_i32(
                            &shader.gradient_point_count,
                            match &cmd.brush {
                                Brush::Solid(_) => 0,
                                Brush::LinearGradient { stops, .. }
                                | Brush::RadialGradient { stops, .. } => stops.len() as i32,
                            },
                        )
                        .set_f32_slice(
                            &shader.gradient_stops,
                            match &cmd.brush {
                                Brush::Solid(_) => &raw_stops,
                                Brush::LinearGradient { stops, .. }
                                | Brush::RadialGradient { stops, .. } => {
                                    for (i, point) in stops.iter().enumerate() {
                                        raw_stops[i] = point.stop;
                                    }
                                    &raw_stops
                                }
                            },
                        )
                        .set_vector4_slice(
                            &shader.gradient_colors,
                            match &cmd.brush {
                                Brush::Solid(_) => &raw_colors,
                                Brush::LinearGradient { stops, .. }
                                | Brush::RadialGradient { stops, .. } => {
                                    for (i, point) in stops.iter().enumerate() {
                                        raw_colors[i] = point.color.as_frgba();
                                    }
                                    &raw_colors
                                }
                            },
                        )
                        .set_f32(&shader.opacity, cmd.opacity);
                },
            )?;
        }

        state.set_scissor_test(false);

        Ok(statistics)
    }
}
