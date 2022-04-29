//! Engine is container for all subsystems (renderer, ui, sound, resource manager). It also
//! creates a window and an OpenGL context.

#![warn(missing_docs)]

pub mod error;
pub mod executor;
pub mod framework;
pub mod resource_manager;

use crate::{
    asset::ResourceState,
    core::{algebra::Vector2, instant, pool::Handle},
    engine::{
        error::EngineError,
        resource_manager::{container::event::ResourceEvent, ResourceManager},
    },
    event::Event,
    event_loop::EventLoop,
    gui::UserInterface,
    plugin::{Plugin, PluginContext, PluginRegistrationContext},
    renderer::{framework::error::FrameworkError, Renderer},
    resource::{model::Model, texture::TextureKind},
    scene::{
        node::constructor::NodeConstructorContainer, sound::SoundEngine, Scene, SceneContainer,
    },
    script::{constructor::ScriptConstructorContainer, Script, ScriptContext},
    utils::log::Log,
    window::{Window, WindowBuilder},
};
use fyrox_core::futures::executor::block_on;
use std::{
    collections::HashSet,
    sync::{
        mpsc::{channel, Receiver},
        Arc, Mutex,
    },
    time::Duration,
};

/// Serialization context holds runtime type information that allows to create unknown types using
/// their UUIDs and a respective constructors.
pub struct SerializationContext {
    /// A node constructor container.
    pub node_constructors: NodeConstructorContainer,
    /// A script constructor container.
    pub script_constructors: ScriptConstructorContainer,
}

impl Default for SerializationContext {
    fn default() -> Self {
        Self::new()
    }
}

impl SerializationContext {
    /// Creates default serialization context.
    pub fn new() -> Self {
        Self {
            node_constructors: NodeConstructorContainer::new(),
            script_constructors: ScriptConstructorContainer::new(),
        }
    }
}

/// See module docs.
pub struct Engine {
    #[cfg(not(target_arch = "wasm32"))]
    context: glutin::WindowedContext<glutin::PossiblyCurrent>,
    #[cfg(target_arch = "wasm32")]
    window: winit::window::Window,
    /// Current renderer. You should call at least [render](Self::render) method to see your scene on
    /// screen.
    pub renderer: Renderer,
    /// User interface allows you to build interface of any kind.
    pub user_interface: UserInterface,
    /// Current resource manager. Resource manager can be cloned (it does clone only ref) to be able to
    /// use resource manager from any thread, this is useful to load resources from multiple
    /// threads to decrease loading times of your game by utilizing all available power of
    /// your CPU.
    pub resource_manager: ResourceManager,
    /// All available scenes in the engine.
    pub scenes: SceneContainer,
    /// The time user interface took for internal needs. TODO: This is not the right place
    /// for such statistics, probably it is best to make separate structure to hold all
    /// such data.
    pub ui_time: Duration,

    model_events_receiver: Receiver<ResourceEvent<Model>>,

    // Sound context control all sound sources in the engine. It is wrapped into Arc<Mutex<>>
    // because internally sound engine spawns separate thread to mix and send data to sound
    // device. For more info see docs for Context.
    sound_engine: Arc<Mutex<SoundEngine>>,

    // A set of plugins used by the engine.
    plugins: Vec<Box<dyn Plugin>>,

    /// A special container that is able to create nodes by their type UUID. Use a copy of this
    /// value whenever you need it as a parameter in other parts of the engine.
    pub serialization_context: Arc<SerializationContext>,
}

struct ResourceGraphVertex {
    resource: Model,
    children: Vec<ResourceGraphVertex>,
    resource_manager: ResourceManager,
}

impl ResourceGraphVertex {
    pub fn new(model: Model, resource_manager: ResourceManager) -> Self {
        let mut children = Vec::new();

        // Look for dependent resources.
        let mut dependent_resources = HashSet::new();
        for other_model in resource_manager.state().containers().models.iter() {
            let state = other_model.state();
            if let ResourceState::Ok(ref model_data) = *state {
                if model_data
                    .get_scene()
                    .graph
                    .linear_iter()
                    .any(|n| n.resource.as_ref().map_or(false, |r| r == &model))
                {
                    dependent_resources.insert(other_model.clone());
                }
            }
        }

        children.extend(
            dependent_resources
                .into_iter()
                .map(|r| ResourceGraphVertex::new(r, resource_manager.clone())),
        );

        Self {
            resource: model,
            children,
            resource_manager,
        }
    }

    pub fn resolve(&self) {
        Log::info(format!(
            "Resolving {} resource from dependency graph...",
            self.resource.state().path().display()
        ));

        block_on(
            self.resource
                .data_ref()
                .get_scene_mut()
                .resolve(self.resource_manager.clone()),
        );

        for child in self.children.iter() {
            child.resolve();
        }
    }
}

struct ResourceDependencyGraph {
    root: ResourceGraphVertex,
}

impl ResourceDependencyGraph {
    pub fn new(model: Model, resource_manager: ResourceManager) -> Self {
        Self {
            root: ResourceGraphVertex::new(model, resource_manager),
        }
    }

    pub fn resolve(&self) {
        self.root.resolve()
    }
}

/// Engine initialization parameters.
pub struct EngineInitParams<'a> {
    /// A window builder.
    pub window_builder: WindowBuilder,
    /// A special container that is able to create nodes by their type UUID.
    pub serialization_context: Arc<SerializationContext>,
    /// A resource manager.
    pub resource_manager: ResourceManager,
    /// OS event loop.
    pub events_loop: &'a EventLoop<()>,
    /// Whether to use vertical synchronization or not. V-sync will force your game to render
    /// frames with the synchronization rate of your monitor (which is ~60 FPS). Keep in mind
    /// vertical synchronization could not be available on your OS and engine might fail to
    /// initialize if v-sync is on.
    pub vsync: bool,
}

impl Engine {
    /// Creates new instance of engine from given initialization parameters.
    ///
    /// Automatically creates all sub-systems (renderer, sound, ui, etc.).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use fyrox::engine::{Engine, EngineInitParams};
    /// use fyrox::window::WindowBuilder;
    /// use fyrox::engine::resource_manager::ResourceManager;
    /// use fyrox::event_loop::EventLoop;
    /// use std::sync::Arc;
    /// use fyrox::engine::SerializationContext;
    ///
    /// let evt = EventLoop::new();
    /// let window_builder = WindowBuilder::new()
    ///     .with_title("Test")
    ///     .with_fullscreen(None);
    /// let serialization_context = Arc::new(SerializationContext::new());
    /// let mut engine = Engine::new(EngineInitParams {
    ///     window_builder,
    ///     resource_manager: ResourceManager::new(serialization_context.clone()),
    ///     serialization_context,
    ///     events_loop: &evt,
    ///     vsync: false,
    /// })
    /// .unwrap();
    /// ```
    #[inline]
    #[allow(unused_variables)]
    pub fn new(params: EngineInitParams) -> Result<Self, EngineError> {
        let EngineInitParams {
            window_builder,
            serialization_context: node_constructors,
            resource_manager,
            events_loop,
            vsync,
        } = params;

        #[cfg(not(target_arch = "wasm32"))]
        let (context, client_size) = {
            let context_wrapper: glutin::WindowedContext<glutin::NotCurrent> =
                glutin::ContextBuilder::new()
                    .with_vsync(vsync)
                    .with_gl_profile(glutin::GlProfile::Core)
                    .with_gl(glutin::GlRequest::GlThenGles {
                        opengl_version: (3, 3),
                        opengles_version: (3, 0),
                    })
                    .build_windowed(window_builder, events_loop)?;

            let ctx = match unsafe { context_wrapper.make_current() } {
                Ok(context) => context,
                Err((_, e)) => return Err(EngineError::from(e)),
            };
            let inner_size = ctx.window().inner_size();
            (
                ctx,
                Vector2::new(inner_size.width as f32, inner_size.height as f32),
            )
        };

        #[cfg(target_arch = "wasm32")]
        let (window, client_size, glow_context) = {
            let winit_window = window_builder.build(events_loop).unwrap();

            use crate::core::wasm_bindgen::JsCast;
            use crate::platform::web::WindowExtWebSys;

            let canvas = winit_window.canvas();

            let window = crate::core::web_sys::window().unwrap();
            let document = window.document().unwrap();
            let body = document.body().unwrap();

            body.append_child(&canvas)
                .expect("Append canvas to HTML body");

            let webgl2_context = canvas
                .get_context("webgl2")
                .unwrap()
                .unwrap()
                .dyn_into::<crate::core::web_sys::WebGl2RenderingContext>()
                .unwrap();
            let glow_context = glow::Context::from_webgl2_context(webgl2_context);

            let inner_size = winit_window.inner_size();
            (
                winit_window,
                Vector2::new(inner_size.width as f32, inner_size.height as f32),
                glow_context,
            )
        };

        #[cfg(not(target_arch = "wasm32"))]
        let glow_context =
            { unsafe { glow::Context::from_loader_function(|s| context.get_proc_address(s)) } };

        let sound_engine = SoundEngine::new();

        let renderer = Renderer::new(
            glow_context,
            (client_size.x as u32, client_size.y as u32),
            &resource_manager,
        )?;

        let (rx, tx) = channel();
        resource_manager
            .state()
            .containers_mut()
            .models
            .event_broadcaster
            .add(rx);

        Ok(Self {
            model_events_receiver: tx,
            resource_manager,
            renderer,
            scenes: SceneContainer::new(sound_engine.clone()),
            sound_engine,
            user_interface: UserInterface::new(client_size),
            ui_time: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            context,
            #[cfg(target_arch = "wasm32")]
            window,
            plugins: Default::default(),
            serialization_context: node_constructors,
        })
    }

    /// Adjust size of the frame to be rendered. Must be called after the window size changes.
    /// Will update the renderer and GL context frame size.
    /// When using the [`framework::Framework`], you don't need to call this yourself.
    pub fn set_frame_size(&mut self, new_size: (u32, u32)) -> Result<(), FrameworkError> {
        self.renderer.set_frame_size(new_size)?;

        #[cfg(not(target_arch = "wasm32"))]
        self.context.resize(new_size.into());

        Ok(())
    }

    /// Returns reference to main window. Could be useful to set fullscreen mode, change
    /// size of window, its title, etc.
    #[inline]
    pub fn get_window(&self) -> &Window {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.context.window()
        }
        #[cfg(target_arch = "wasm32")]
        {
            &self.window
        }
    }

    /// Performs single update tick with given time delta. Engine internally will perform update
    /// of all scenes, sub-systems, user interface, etc. Must be called in order to get engine
    /// functioning.
    pub fn update(&mut self, dt: f32) {
        self.pre_update(dt);
        self.post_update(dt);
    }

    /// Performs pre update for the engine.
    ///
    /// Normally, this is called from `Engine::update()`.
    /// You should only call this manually if you don't use that method.
    pub fn pre_update(&mut self, dt: f32) {
        let inner_size = self.get_window().inner_size();
        let window_size = Vector2::new(inner_size.width as f32, inner_size.height as f32);

        self.resource_manager.state().update(dt);
        self.renderer.update_caches(dt);
        self.handle_model_events();

        for scene in self.scenes.iter_mut().filter(|s| s.enabled) {
            let frame_size = scene.render_target.as_ref().map_or(window_size, |rt| {
                if let TextureKind::Rectangle { width, height } = rt.data_ref().kind() {
                    Vector2::new(width as f32, height as f32)
                } else {
                    panic!("only rectangle textures can be used as render target!");
                }
            });

            scene.update(frame_size, dt);
        }
    }

    /// Performs post update for the engine.
    ///
    /// Normally, this is called from `Engine::update()`.
    /// You should only call this manually if you don't use that method.
    pub fn post_update(&mut self, dt: f32) {
        let inner_size = self.get_window().inner_size();
        let window_size = Vector2::new(inner_size.width as f32, inner_size.height as f32);

        let time = instant::Instant::now();
        self.user_interface.update(window_size, dt);
        self.ui_time = instant::Instant::now() - time;
    }

    /// Performs update of every plugin.
    ///
    /// # Important notes
    ///
    /// This method is intended to be used by the editor and game runner. If you're using the
    /// engine as a framework, then you should not call this method because you'll most likely
    /// do something wrong.
    pub fn update_plugins(&mut self, dt: f32, is_in_editor: bool) {
        let mut context = PluginContext {
            is_in_editor,
            scenes: &mut self.scenes,
            resource_manager: &self.resource_manager,
            renderer: &mut self.renderer,
            dt,
            serialization_context: self.serialization_context.clone(),
        };

        for plugin in self.plugins.iter_mut() {
            plugin.update(&mut context);
        }
    }

    /// Calls [`Plugin::on_unload`] of every plugin.
    ///
    /// # Important notes
    ///
    /// This method is intended to be used by the editor and game runner. If you're using the
    /// engine as a framework, then you should not call this method because you'll most likely
    /// do something wrong.
    pub fn unload_plugins(&mut self, dt: f32, is_in_editor: bool) {
        let mut context = PluginContext {
            is_in_editor,
            scenes: &mut self.scenes,
            resource_manager: &self.resource_manager,
            renderer: &mut self.renderer,
            dt,
            serialization_context: self.serialization_context.clone(),
        };

        for plugin in self.plugins.iter_mut() {
            plugin.on_unload(&mut context);
        }
    }

    /// Processes an OS event by every registered plugin.
    pub fn handle_os_event_by_plugins(&mut self, event: &Event<()>, dt: f32, is_in_editor: bool) {
        for plugin in self.plugins.iter_mut() {
            plugin.on_os_event(
                event,
                PluginContext {
                    is_in_editor,
                    scenes: &mut self.scenes,
                    resource_manager: &self.resource_manager,
                    renderer: &mut self.renderer,
                    dt,
                    serialization_context: self.serialization_context.clone(),
                },
            );
        }
    }

    /// Calls [`Plugin::on_enter_play_mode`] for every plugin.
    pub fn call_plugins_on_enter_play_mode(
        &mut self,
        scene: Handle<Scene>,
        dt: f32,
        is_in_editor: bool,
    ) {
        for plugin in self.plugins.iter_mut() {
            plugin.on_enter_play_mode(
                scene,
                PluginContext {
                    is_in_editor,
                    scenes: &mut self.scenes,
                    resource_manager: &self.resource_manager,
                    renderer: &mut self.renderer,
                    dt,
                    serialization_context: self.serialization_context.clone(),
                },
            );
        }
    }

    /// Calls [`Plugin::on_leave_play_mode`] for every plugin.
    pub fn call_plugins_on_leave_play_mode(&mut self, dt: f32, is_in_editor: bool) {
        for plugin in self.plugins.iter_mut() {
            plugin.on_leave_play_mode(PluginContext {
                is_in_editor,
                scenes: &mut self.scenes,
                resource_manager: &self.resource_manager,
                renderer: &mut self.renderer,
                dt,
                serialization_context: self.serialization_context.clone(),
            });
        }
    }

    pub(crate) fn process_scripts<T>(&mut self, scene: Handle<Scene>, dt: f32, mut func: T)
    where
        T: FnMut(&mut Script, ScriptContext),
    {
        let scene = &mut self.scenes[scene];

        // Iterate over the nodes without borrowing, we'll move data around to solve borrowing issues.
        for node_index in 0..scene.graph.capacity() {
            let handle = scene.graph.handle_from_index(node_index);

            // We're interested only in nodes with scripts.
            if scene
                .graph
                .try_get(handle)
                .map_or(true, |node| node.script.is_none())
            {
                continue;
            }

            // If a node has script assigned, then temporarily move it out of the pool with taking
            // the ownership to satisfy borrow checker. Moving a node out of the pool is fast, because
            // it is just a copy of 16 bytes which can be performed in a single instruction on modern
            // CPUs.
            let (ticket, mut node) = scene.graph.take_reserve_internal(handle);

            // Take the script off the node to get mutable borrow to it without mutably borrowing
            // the node itself. This operation is fast as well.
            let mut script = node.script.take().unwrap();

            // Find respective plugin.
            if let Some(plugin) = self
                .plugins
                .iter_mut()
                .find(|p| p.id() == script.plugin_uuid())
            {
                // Form the context with all available data.
                let context = ScriptContext {
                    dt,
                    plugin: &mut **plugin,
                    node: &mut node,
                    handle,
                    scene,
                    resource_manager: &self.resource_manager,
                };

                func(&mut script, context);
            }

            // Put the script back to the node.
            node.script = Some(script);

            // Put the node back in the graph.
            scene.graph.put_back_internal(ticket, node);
        }
    }

    /// Updates scripts of specified scene. It must be called manually! Usually the editor
    /// calls this for you when it is in the play mode.
    ///
    /// # Important notes
    ///
    /// This method is intended to be used by the editor and game runner. If you're using the
    /// engine as a framework, then you should not call this method because you'll most likely
    /// do something wrong.
    pub fn update_scene_scripts(&mut self, scene: Handle<Scene>, dt: f32) {
        self.process_scripts(scene, dt, |script, context| script.on_update(context));
    }

    /// Passes specified OS event to every script of the specified scene.
    ///
    /// # Important notes
    ///
    /// This method is intended to be used by the editor and game runner. If you're using the
    /// engine as a framework, then you should not call this method because you'll most likely
    /// do something wrong.
    pub fn handle_os_event_by_scripts(&mut self, event: &Event<()>, scene: Handle<Scene>, dt: f32) {
        self.process_scripts(scene, dt, |script, context| {
            script.on_os_event(event, context)
        })
    }

    /// Initializes every script in the scene.
    ///
    ///
    /// # Important notes
    ///
    /// This method is intended to be used by the editor and game runner. If you're using the
    /// engine as a framework, then you should not call this method because you'll most likely
    /// do something wrong.
    pub fn initialize_scene_scripts(&mut self, scene: Handle<Scene>, dt: f32) {
        self.process_scripts(scene, dt, |script, context| script.on_init(context))
    }

    /// Handle hot-reloading of resources.
    ///
    /// Normally, this is called from `Engine::update()`.
    /// You should only call this manually if you don't use that method.
    pub fn handle_model_events(&mut self) {
        while let Ok(event) = self.model_events_receiver.try_recv() {
            if let ResourceEvent::Reloaded(model) = event {
                Log::info(format!(
                    "A model resource {} was reloaded, propagating changes...",
                    model.state().path().display()
                ));

                // Build resource dependency graph and resolve it first.
                ResourceDependencyGraph::new(model, self.resource_manager.clone()).resolve();

                Log::info("Propagating changes to active scenes...".to_string());

                // Resolve all scenes.
                // TODO: This might be inefficient if there is bunch of scenes loaded,
                // however this seems to be very rare case so it should be ok.
                for scene in self.scenes.iter_mut() {
                    block_on(scene.resolve(self.resource_manager.clone()));
                }
            }
        }
    }

    /// Performs rendering of single frame, must be called from your game loop, otherwise you won't
    /// see anything.
    #[inline]
    pub fn render(&mut self) -> Result<(), FrameworkError> {
        self.user_interface.draw();

        #[cfg(not(target_arch = "wasm32"))]
        {
            self.renderer.render_and_swap_buffers(
                &self.scenes,
                self.user_interface.get_drawing_context(),
                &self.context,
            )
        }
        #[cfg(target_arch = "wasm32")]
        {
            self.renderer
                .render_and_swap_buffers(&self.scenes, &self.user_interface.get_drawing_context())
        }
    }

    /// Sets master gain of the sound engine. Can be used to control overall gain of all sound
    /// scenes at once.
    pub fn set_sound_gain(&mut self, gain: f32) {
        self.sound_engine.lock().unwrap().set_master_gain(gain);
    }

    /// Returns master gain of the sound engine.
    pub fn sound_gain(&self) -> f32 {
        self.sound_engine.lock().unwrap().master_gain()
    }

    /// Adds new plugin.
    pub fn add_plugin<P>(&mut self, mut plugin: P, is_in_editor: bool, init: bool)
    where
        P: Plugin,
    {
        plugin.on_register(PluginRegistrationContext {
            serialization_context: self.serialization_context.clone(),
        });

        if init {
            plugin.on_standalone_init(PluginContext {
                is_in_editor,
                scenes: &mut self.scenes,
                resource_manager: &self.resource_manager,
                renderer: &mut self.renderer,
                dt: 0.0,
                serialization_context: self.serialization_context.clone(),
            });
        }

        self.plugins.push(Box::new(plugin));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.unload_plugins(0.0, false);
    }
}
