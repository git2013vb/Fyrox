//! Example 12. Custom loader
//!
//! Difficulty: Moderate.
//!
//! This example shows how to create a custom loader. It is a very basic example and in future it should be improved by
//! writing some more complex loader such as loading a model from ply or obj file.

/// For simplicity we just simply wrap the default loader and log the invocation t
pub mod shared;

use crate::shared::create_camera;
use fyrox::{
    core::{
        algebra::{Matrix4, Vector3},
        color::Color,
        instant::Instant,
        parking_lot::Mutex,
        sstorage::ImmutableString,
    },
    engine::{
        resource_manager::{
            container::event::ResourceEventBroadcaster,
            loader::{
                model::ModelLoader, texture::TextureLoader, BoxedLoaderFuture, ResourceLoader,
            },
            ResourceManager,
        },
        Engine, EngineInitParams, SerializationContext,
    },
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    material::{shader::SamplerFallback, Material, PropertyValue},
    resource::{
        model::{Model, ModelImportOptions},
        texture::{Texture, TextureImportOptions},
    },
    scene::{
        base::BaseBuilder,
        light::{point::PointLightBuilder, BaseLightBuilder},
        mesh::{
            surface::{SurfaceBuilder, SurfaceData},
            MeshBuilder,
        },
        transform::TransformBuilder,
        Scene,
    },
    utils::{
        log::{Log, MessageKind},
        translate_event,
    },
    window::WindowBuilder,
};
use fyrox_core::futures::executor::block_on;
use std::sync::Arc;

struct CustomModelLoader(Arc<ModelLoader>);

impl ResourceLoader<Model, ModelImportOptions> for CustomModelLoader {
    fn load(
        &self,
        resource: Model,
        default_import_options: ModelImportOptions,
        event_broadcaster: ResourceEventBroadcaster<Model>,
        reload: bool,
    ) -> BoxedLoaderFuture {
        // Arc is required as BoxedLoaderFuture has a static lifetime and hence self cannot be
        // moved into an async block.
        let loader = self.0.clone();

        Box::pin(async move {
            println!("CUSTOM LOADER: loading model {:?}", resource.state().path());
            loader
                .load(resource, default_import_options, event_broadcaster, reload)
                .await
        })
    }
}

/// For simplicity we just simply wrap the default loader and log the invocation to the console.
struct CustomTextureLoader(Arc<TextureLoader>);

impl ResourceLoader<Texture, TextureImportOptions> for CustomTextureLoader {
    fn load(
        &self,
        resource: Texture,
        default_import_options: TextureImportOptions,
        event_broadcaster: ResourceEventBroadcaster<Texture>,
        reload: bool,
    ) -> BoxedLoaderFuture {
        // Arc is required as BoxedLoaderFuture has a static lifetime and hence self cannot be
        // moved into an async block.
        let loader = self.0.clone();

        Box::pin(async move {
            println!(
                "CUSTOM LOADER: loading texture {:?}",
                resource.state().path()
            );
            loader
                .load(resource, default_import_options, event_broadcaster, reload)
                .await
        })
    }
}

struct GameSceneLoader {
    scene: Scene,
}

impl GameSceneLoader {
    async fn load_with(resource_manager: ResourceManager) -> Self {
        let mut scene = Scene::new();

        // Set ambient light.
        scene.ambient_lighting_color = Color::opaque(200, 200, 200);

        // Camera is our eyes in the world - you won't see anything without it.
        create_camera(
            resource_manager.clone(),
            Vector3::new(0.0, 6.0, -12.0),
            &mut scene.graph,
        )
        .await;

        // Add some light.
        PointLightBuilder::new(BaseLightBuilder::new(
            BaseBuilder::new().with_local_transform(
                TransformBuilder::new()
                    .with_local_position(Vector3::new(0.0, 12.0, 0.0))
                    .build(),
            ),
        ))
        .with_radius(20.0)
        .build(&mut scene.graph);

        // Add some model with animation.
        let model_resource = resource_manager
            .request_model("examples/data/mutant/mutant.FBX")
            .await;

        let model_handle = model_resource.unwrap().instantiate_geometry(&mut scene);
        scene.graph[model_handle]
            .local_transform_mut()
            .set_scale(Vector3::new(0.05, 0.05, 0.05));

        let mut material = Material::standard();
        material
            .set_property(
                &ImmutableString::new("diffuseTexture"),
                PropertyValue::Sampler {
                    value: Some(resource_manager.request_texture("examples/data/concrete2.dds")),
                    fallback: SamplerFallback::White,
                },
            )
            .unwrap();

        // Add floor.
        MeshBuilder::new(
            BaseBuilder::new().with_local_transform(
                TransformBuilder::new()
                    .with_local_position(Vector3::new(0.0, -0.25, 0.0))
                    .build(),
            ),
        )
        .with_surfaces(vec![SurfaceBuilder::new(Arc::new(Mutex::new(
            SurfaceData::make_cube(Matrix4::new_nonuniform_scaling(&Vector3::new(
                25.0, 0.25, 25.0,
            ))),
        )))
        .with_material(Arc::new(Mutex::new(material)))
        .build()])
        .build(&mut scene.graph);

        Self { scene }
    }
}

fn main() {
    let event_loop = EventLoop::new();

    let window_builder = WindowBuilder::new()
        .with_title("Example 12 - Custom resource loader")
        .with_resizable(true);

    let serialization_context = Arc::new(SerializationContext::new());
    let resource_manager = ResourceManager::new(serialization_context.clone());

    // Set up our custom loaders
    {
        let mut state = resource_manager.state();
        let containers = state.containers_mut();
        containers.set_model_loader(CustomModelLoader(Arc::new(ModelLoader {
            resource_manager: resource_manager.clone(),
            serialization_context: serialization_context.clone(),
        })));
        containers.set_texture_loader(CustomTextureLoader(Arc::new(TextureLoader)));
    }

    let mut engine = Engine::new(EngineInitParams {
        window_builder,
        resource_manager: ResourceManager::new(serialization_context.clone()),
        serialization_context,
        events_loop: &event_loop,
        vsync: false,
    })
    .unwrap();

    let scene = block_on(GameSceneLoader::load_with(engine.resource_manager.clone())).scene;
    engine.scenes.add(scene);

    let clock = Instant::now();
    let fixed_timestep = 1.0 / 60.0;
    let mut elapsed_time = 0.0;

    event_loop.run(move |event, _, control_flow| match event {
        Event::MainEventsCleared => {
            let mut dt = clock.elapsed().as_secs_f32() - elapsed_time;
            while dt >= fixed_timestep {
                dt -= fixed_timestep;
                elapsed_time += fixed_timestep;

                engine.update(fixed_timestep, control_flow);
            }

            engine.get_window().request_redraw();
        }
        Event::RedrawRequested(_) => {
            engine.render().unwrap();
        }
        Event::WindowEvent { event, .. } => {
            match event {
                WindowEvent::CloseRequested => *control_flow = ControlFlow::Exit,
                WindowEvent::Resized(size) => {
                    if let Err(e) = engine.set_frame_size(size.into()) {
                        Log::writeln(
                            MessageKind::Error,
                            format!("Unable to set frame size: {:?}", e),
                        );
                    }
                }
                _ => (),
            }

            if let Some(os_event) = translate_event(&event) {
                engine.user_interface.process_os_event(&os_event);
            }
        }
        _ => *control_flow = ControlFlow::Poll,
    });
}
