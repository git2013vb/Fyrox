//! GBuffer Layout:
//!
//! RT0: sRGBA8 - Diffuse color (xyz)
//! RT1: RGBA8 - Normal (xyz)
//! RT2: RGBA16F - Ambient light + emission (both in xyz)
//! RT3: RGBA8 - Metallic (x) + Roughness (y) + Ambient Occlusion (z)
//! RT4: R8UI - Decal mask (x)
//!
//! Every alpha channel is used for layer blending for terrains. This is inefficient, but for
//! now I don't know better solution.

use crate::core::sstorage::ImmutableString;
use crate::renderer::framework::framebuffer::BlendParameters;
use crate::renderer::framework::geometry_buffer::{GeometryBuffer, GeometryBufferKind};
use crate::scene::decal::Decal;
use crate::{
    core::{
        algebra::{Matrix4, Vector2},
        color::Color,
        math::Rect,
        scope_profile,
    },
    renderer::{
        apply_material,
        batch::BatchStorage,
        cache::shader::ShaderCache,
        framework::{
            error::FrameworkError,
            framebuffer::{Attachment, AttachmentKind, DrawParameters, FrameBuffer},
            gpu_program::GpuProgramBinding,
            gpu_texture::{
                Coordinate, GpuTexture, GpuTextureKind, MagnificationFilter, MinificationFilter,
                PixelKind, WrapMode,
            },
            state::{BlendFactor, BlendFunc, PipelineState},
        },
        gbuffer::decal::DecalShader,
        GeometryCache, MaterialContext, RenderPassStatistics, TextureCache,
    },
    scene::{camera::Camera, graph::Graph, mesh::surface::SurfaceData, mesh::RenderPath},
};
use std::{cell::RefCell, rc::Rc};

mod decal;

pub struct GBuffer {
    framebuffer: FrameBuffer,
    decal_framebuffer: FrameBuffer,
    pub width: i32,
    pub height: i32,
    cube: GeometryBuffer,
    decal_shader: DecalShader,
    render_pass_name: ImmutableString,
}

pub(crate) struct GBufferRenderContext<'a, 'b> {
    pub state: &'a mut PipelineState,
    pub camera: &'b Camera,
    pub geom_cache: &'a mut GeometryCache,
    pub batch_storage: &'a BatchStorage,
    pub texture_cache: &'a mut TextureCache,
    pub shader_cache: &'a mut ShaderCache,
    #[allow(dead_code)]
    pub environment_dummy: Rc<RefCell<GpuTexture>>,
    pub white_dummy: Rc<RefCell<GpuTexture>>,
    pub normal_dummy: Rc<RefCell<GpuTexture>>,
    pub black_dummy: Rc<RefCell<GpuTexture>>,
    pub use_parallax_mapping: bool,
    pub graph: &'b Graph,
}

impl GBuffer {
    pub fn new(
        state: &mut PipelineState,
        width: usize,
        height: usize,
    ) -> Result<Self, FrameworkError> {
        scope_profile!();

        let mut depth_stencil_texture = GpuTexture::new(
            state,
            GpuTextureKind::Rectangle { width, height },
            PixelKind::D24S8,
            MinificationFilter::Nearest,
            MagnificationFilter::Nearest,
            1,
            None,
        )?;
        depth_stencil_texture
            .bind_mut(state, 0)
            .set_wrap(Coordinate::S, WrapMode::ClampToEdge)
            .set_wrap(Coordinate::T, WrapMode::ClampToEdge);

        let depth_stencil = Rc::new(RefCell::new(depth_stencil_texture));

        let mut diffuse_texture = GpuTexture::new(
            state,
            GpuTextureKind::Rectangle { width, height },
            PixelKind::SRGBA8,
            MinificationFilter::Nearest,
            MagnificationFilter::Nearest,
            1,
            None,
        )?;
        diffuse_texture
            .bind_mut(state, 0)
            .set_wrap(Coordinate::S, WrapMode::ClampToEdge)
            .set_wrap(Coordinate::T, WrapMode::ClampToEdge);
        let diffuse_texture = Rc::new(RefCell::new(diffuse_texture));

        let mut normal_texture = GpuTexture::new(
            state,
            GpuTextureKind::Rectangle { width, height },
            PixelKind::RGBA8,
            MinificationFilter::Nearest,
            MagnificationFilter::Nearest,
            1,
            None,
        )?;
        normal_texture
            .bind_mut(state, 0)
            .set_wrap(Coordinate::S, WrapMode::ClampToEdge)
            .set_wrap(Coordinate::T, WrapMode::ClampToEdge);
        let normal_texture = Rc::new(RefCell::new(normal_texture));

        let mut ambient_texture = GpuTexture::new(
            state,
            GpuTextureKind::Rectangle { width, height },
            PixelKind::RGBA16F,
            MinificationFilter::Nearest,
            MagnificationFilter::Nearest,
            1,
            None,
        )?;
        ambient_texture
            .bind_mut(state, 0)
            .set_wrap(Coordinate::S, WrapMode::ClampToEdge)
            .set_wrap(Coordinate::T, WrapMode::ClampToEdge);

        let mut decal_mask_texture = GpuTexture::new(
            state,
            GpuTextureKind::Rectangle { width, height },
            PixelKind::R8UI,
            MinificationFilter::Nearest,
            MagnificationFilter::Nearest,
            1,
            None,
        )?;
        decal_mask_texture
            .bind_mut(state, 0)
            .set_wrap(Coordinate::S, WrapMode::ClampToEdge)
            .set_wrap(Coordinate::T, WrapMode::ClampToEdge);

        let mut material_texture = GpuTexture::new(
            state,
            GpuTextureKind::Rectangle { width, height },
            PixelKind::RGBA8,
            MinificationFilter::Nearest,
            MagnificationFilter::Nearest,
            1,
            None,
        )?;
        material_texture
            .bind_mut(state, 0)
            .set_wrap(Coordinate::S, WrapMode::ClampToEdge)
            .set_wrap(Coordinate::T, WrapMode::ClampToEdge);

        let framebuffer = FrameBuffer::new(
            state,
            Some(Attachment {
                kind: AttachmentKind::DepthStencil,
                texture: depth_stencil,
            }),
            vec![
                Attachment {
                    kind: AttachmentKind::Color,
                    texture: diffuse_texture.clone(),
                },
                Attachment {
                    kind: AttachmentKind::Color,
                    texture: normal_texture.clone(),
                },
                Attachment {
                    kind: AttachmentKind::Color,
                    texture: Rc::new(RefCell::new(ambient_texture)),
                },
                Attachment {
                    kind: AttachmentKind::Color,
                    texture: Rc::new(RefCell::new(material_texture)),
                },
                Attachment {
                    kind: AttachmentKind::Color,
                    texture: Rc::new(RefCell::new(decal_mask_texture)),
                },
            ],
        )?;

        let decal_framebuffer = FrameBuffer::new(
            state,
            None,
            vec![
                Attachment {
                    kind: AttachmentKind::Color,
                    texture: diffuse_texture,
                },
                Attachment {
                    kind: AttachmentKind::Color,
                    texture: normal_texture,
                },
            ],
        )?;

        Ok(Self {
            framebuffer,
            width: width as i32,
            height: height as i32,
            decal_shader: DecalShader::new(state)?,
            cube: GeometryBuffer::from_surface_data(
                &SurfaceData::make_cube(Matrix4::identity()),
                GeometryBufferKind::StaticDraw,
                state,
            ),
            decal_framebuffer,
            render_pass_name: ImmutableString::new("GBuffer"),
        })
    }

    pub fn framebuffer(&self) -> &FrameBuffer {
        &self.framebuffer
    }

    pub fn depth(&self) -> Rc<RefCell<GpuTexture>> {
        self.framebuffer.depth_attachment().unwrap().texture.clone()
    }

    pub fn diffuse_texture(&self) -> Rc<RefCell<GpuTexture>> {
        self.framebuffer.color_attachments()[0].texture.clone()
    }

    pub fn normal_texture(&self) -> Rc<RefCell<GpuTexture>> {
        self.framebuffer.color_attachments()[1].texture.clone()
    }

    pub fn ambient_texture(&self) -> Rc<RefCell<GpuTexture>> {
        self.framebuffer.color_attachments()[2].texture.clone()
    }

    pub fn material_texture(&self) -> Rc<RefCell<GpuTexture>> {
        self.framebuffer.color_attachments()[3].texture.clone()
    }

    pub fn decal_mask_texture(&self) -> Rc<RefCell<GpuTexture>> {
        self.framebuffer.color_attachments()[4].texture.clone()
    }

    #[must_use]
    pub(crate) fn fill(&mut self, args: GBufferRenderContext) -> RenderPassStatistics {
        scope_profile!();

        let mut statistics = RenderPassStatistics::default();

        let GBufferRenderContext {
            state,
            camera,
            geom_cache,
            batch_storage,
            texture_cache,
            shader_cache,
            use_parallax_mapping,
            white_dummy,
            normal_dummy,
            black_dummy,
            graph,
            ..
        } = args;

        let viewport = Rect::new(0, 0, self.width, self.height);
        self.framebuffer.clear(
            state,
            viewport,
            Some(Color::from_rgba(0, 0, 0, 0)),
            Some(1.0),
            Some(0),
        );

        let initial_view_projection = camera.view_projection_matrix();

        for batch in batch_storage
            .batches
            .iter()
            .filter(|b| b.render_path == RenderPath::Deferred)
        {
            let material = batch.material.lock();
            let geometry = geom_cache.get(state, &batch.data);

            if let Some(render_pass) = shader_cache
                .get(state, material.shader())
                .and_then(|shader_set| shader_set.render_passes.get(&self.render_pass_name))
            {
                for instance in batch.instances.iter() {
                    if camera.visibility_cache.is_visible(instance.owner) {
                        let apply_uniforms = |mut program_binding: GpuProgramBinding| {
                            let view_projection = if instance.depth_offset != 0.0 {
                                let mut projection = camera.projection_matrix();
                                projection[14] -= instance.depth_offset;
                                projection * camera.view_matrix()
                            } else {
                                initial_view_projection
                            };

                            apply_material(MaterialContext {
                                material: &material,
                                program_binding: &mut program_binding,
                                texture_cache,
                                world_matrix: &instance.world_transform,
                                wvp_matrix: &(view_projection * instance.world_transform),
                                bone_matrices: &instance.bone_matrices,
                                use_skeletal_animation: batch.is_skinned,
                                camera_position: &camera.global_position(),
                                use_pom: use_parallax_mapping,
                                light_position: &Default::default(),
                                normal_dummy: normal_dummy.clone(),
                                white_dummy: white_dummy.clone(),
                                black_dummy: black_dummy.clone(),
                            });
                        };

                        statistics += self.framebuffer.draw(
                            geometry,
                            state,
                            viewport,
                            &render_pass.program,
                            &render_pass.draw_params,
                            apply_uniforms,
                        );
                    }
                }
            }
        }

        let inv_view_proj = initial_view_projection.try_inverse().unwrap_or_default();
        let depth = self.depth();
        let decal_mask = self.decal_mask_texture();
        let resolution = Vector2::new(self.width as f32, self.height as f32);

        // Render decals after because we need to modify diffuse texture of G-Buffer and use depth texture
        // for rendering. We'll render in the G-Buffer, but depth will be used from final frame, since
        // decals do not modify depth (only diffuse and normal maps).
        let unit_cube = &self.cube;
        for decal in graph.linear_iter().filter_map(|n| n.cast::<Decal>()) {
            let shader = &self.decal_shader;
            let program = &self.decal_shader.program;

            let diffuse_texture = decal
                .diffuse_texture()
                .and_then(|t| texture_cache.get(state, t))
                .unwrap_or_else(|| white_dummy.clone());

            let normal_texture = decal
                .normal_texture()
                .and_then(|t| texture_cache.get(state, t))
                .unwrap_or_else(|| normal_dummy.clone());

            let world_view_proj = initial_view_projection * decal.global_transform();

            statistics += self.decal_framebuffer.draw(
                unit_cube,
                state,
                viewport,
                program,
                &DrawParameters {
                    cull_face: None,
                    color_write: Default::default(),
                    depth_write: false,
                    stencil_test: None,
                    depth_test: false,
                    blend: Some(BlendParameters {
                        func: BlendFunc::new(BlendFactor::SrcAlpha, BlendFactor::OneMinusSrcAlpha),
                        ..Default::default()
                    }),
                    stencil_op: Default::default(),
                },
                |mut program_binding| {
                    program_binding
                        .set_matrix4(&shader.world_view_projection, &world_view_proj)
                        .set_matrix4(&shader.inv_view_proj, &inv_view_proj)
                        .set_matrix4(
                            &shader.inv_world_decal,
                            &decal.global_transform().try_inverse().unwrap_or_default(),
                        )
                        .set_vector2(&shader.resolution, &resolution)
                        .set_texture(&shader.scene_depth, &depth)
                        .set_texture(&shader.diffuse_texture, &diffuse_texture)
                        .set_texture(&shader.normal_texture, &normal_texture)
                        .set_texture(&shader.decal_mask, &decal_mask)
                        .set_u32(&shader.layer_index, decal.layer() as u32)
                        .set_linear_color(&shader.color, &decal.color());
                },
            );
        }

        statistics
    }
}
