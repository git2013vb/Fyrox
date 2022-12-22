//! Example 01. Simple scene.
//!
//! Difficulty: Easy.
//!
//! This example shows how to create simple scene with animated model.

pub mod shared;

use crate::shared::create_camera;
use fyrox::{
    core::{
        algebra::{Matrix4, UnitQuaternion, Vector3},
        color::Color,
        pool::Handle,
        sstorage::ImmutableString,
    },
    engine::{executor::Executor, resource_manager::ResourceManager},
    event::{ElementState, Event, VirtualKeyCode, WindowEvent},
    event_loop::ControlFlow,
    gui::{
        message::MessageDirection,
        text::{TextBuilder, TextMessage},
        widget::WidgetBuilder,
        UiNode,
    },
    material::{shader::SamplerFallback, Material, PropertyValue, SharedMaterial},
    plugin::{Plugin, PluginConstructor, PluginContext},
    scene::{
        base::BaseBuilder,
        light::{point::PointLightBuilder, BaseLightBuilder},
        mesh::{
            surface::{SurfaceBuilder, SurfaceData, SurfaceSharedData},
            MeshBuilder,
        },
        node::Node,
        transform::TransformBuilder,
        Scene,
    },
};

struct GameSceneLoader {
    scene: Scene,
    model_handle: Handle<Node>,
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

        // Load model and animation resource in parallel. Is does *not* adds anything to
        // our scene - it just loads a resource then can be used later on to instantiate
        // models from it on scene. Why loading of resource is separated from instantiation?
        // Because it is too inefficient to load a resource every time you trying to
        // create instance of it - much more efficient is to load it once and then make copies
        // of it. In case of models it is very efficient because single vertex and index buffer
        // can be used for all models instances, so memory footprint on GPU will be lower.
        let (model_resource, walk_animation_resource) = fyrox::core::futures::join!(
            resource_manager.request_model("examples/data/mutant/mutant.FBX",),
            resource_manager.request_model("examples/data/mutant/walk.fbx",)
        );

        // Instantiate model on scene - but only geometry, without any animations.
        // Instantiation is a process of embedding model resource data in desired scene.
        let model_handle = model_resource.unwrap().instantiate(&mut scene);

        // Now we have whole sub-graph instantiated, we can start modifying model instance.
        scene.graph[model_handle]
            .local_transform_mut()
            // Our model is too big, fix it by scale.
            .set_scale(Vector3::new(0.05, 0.05, 0.05));

        // Add simple animation for our model. Animations are loaded from model resources -
        // this is because animation is a set of skeleton bones with their own transforms.
        // Once animation resource is loaded it must be re-targeted to our model instance.
        // Why? Because animation in *resource* uses information about *resource* bones,
        // not model instance bones, retarget_animations maps animations of each bone on
        // model instance so animation will know about nodes it should operate on.
        walk_animation_resource
            .unwrap()
            .retarget_animations(model_handle, &mut scene.graph);

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
        .with_surfaces(vec![SurfaceBuilder::new(SurfaceSharedData::new(
            SurfaceData::make_cube(Matrix4::new_nonuniform_scaling(&Vector3::new(
                25.0, 0.25, 25.0,
            ))),
        ))
        .with_material(SharedMaterial::new(material))
        .build()])
        .build(&mut scene.graph);

        Self {
            scene,
            model_handle,
        }
    }
}

struct InputController {
    rotate_left: bool,
    rotate_right: bool,
}

struct Game {
    scene: Handle<Scene>,
    model_handle: Handle<Node>,
    input_controller: InputController,
    debug_text: Handle<UiNode>,
    model_angle: f32,
}

impl Plugin for Game {
    fn update(&mut self, context: &mut PluginContext, _control_flow: &mut ControlFlow) {
        let scene = &mut context.scenes[self.scene];

        // Rotate model according to input controller state
        if self.input_controller.rotate_left {
            self.model_angle -= 5.0f32.to_radians();
        } else if self.input_controller.rotate_right {
            self.model_angle += 5.0f32.to_radians();
        }

        scene.graph[self.model_handle]
            .local_transform_mut()
            .set_rotation(UnitQuaternion::from_axis_angle(
                &Vector3::y_axis(),
                self.model_angle,
            ));

        context.user_interface.send_message(TextMessage::text(
            self.debug_text,
            MessageDirection::ToWidget,
            format!(
                "Example 01 - Simple Scene\nUse [A][D] keys to rotate model.\nFPS: {}",
                context.renderer.get_statistics().frames_per_second
            ),
        ));
    }

    fn on_os_event(
        &mut self,
        event: &Event<()>,
        _context: PluginContext,
        _control_flow: &mut ControlFlow,
    ) {
        if let Event::WindowEvent {
            event: WindowEvent::KeyboardInput { input, .. },
            ..
        } = event
        {
            if let Some(key_code) = input.virtual_keycode {
                match key_code {
                    VirtualKeyCode::A => {
                        self.input_controller.rotate_left = input.state == ElementState::Pressed
                    }
                    VirtualKeyCode::D => {
                        self.input_controller.rotate_right = input.state == ElementState::Pressed
                    }
                    _ => (),
                }
            }
        }
    }
}

struct GameConstructor;

impl PluginConstructor for GameConstructor {
    fn create_instance(
        &self,
        _override_scene: Handle<Scene>,
        context: PluginContext,
    ) -> Box<dyn Plugin> {
        let scene = fyrox::core::futures::executor::block_on(GameSceneLoader::load_with(
            context.resource_manager.clone(),
        ));

        Box::new(Game {
            debug_text: TextBuilder::new(WidgetBuilder::new())
                .build(&mut context.user_interface.build_ctx()),
            scene: context.scenes.add(scene.scene),
            model_handle: scene.model_handle,
            // Create input controller - it will hold information about needed actions.
            input_controller: InputController {
                rotate_left: false,
                rotate_right: false,
            },
            // We will rotate model using keyboard input.
            model_angle: 180.0f32.to_radians(),
        })
    }
}

fn main() {
    let mut executor = Executor::new();
    executor.get_window().set_title("Example 01 - Simple");
    executor.add_plugin_constructor(GameConstructor);
    executor.run()
}
