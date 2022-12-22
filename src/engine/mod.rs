//! Engine is container for all subsystems (renderer, ui, sound, resource manager). It also
//! creates a window and an OpenGL context.

#![warn(missing_docs)]

pub mod error;
pub mod executor;
pub mod resource_manager;

use crate::{
    asset::ResourceState,
    core::{algebra::Vector2, futures::executor::block_on, instant, pool::Handle},
    engine::{
        error::EngineError,
        resource_manager::{container::event::ResourceEvent, ResourceManager, ResourceWaitContext},
    },
    event::Event,
    event_loop::{ControlFlow, EventLoop},
    gui::UserInterface,
    plugin::{
        Plugin, PluginConstructor, PluginContext, PluginRegistrationContext, SoundEngineHelper,
    },
    renderer::{framework::error::FrameworkError, Renderer},
    resource::{model::Model, texture::TextureKind},
    scene::{
        base::ScriptMessage, node::constructor::NodeConstructorContainer, sound::SoundEngine,
        Scene, SceneContainer,
    },
    script::{constructor::ScriptConstructorContainer, Script, ScriptContext, ScriptDeinitContext},
    utils::log::Log,
    window::{Window, WindowBuilder},
};
use fxhash::FxHashSet;
use std::{
    collections::{HashSet, VecDeque},
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

    // A set of plugin constructors.
    plugin_constructors: Vec<Box<dyn PluginConstructor>>,

    // A set of plugins used by the engine.
    plugins: Vec<Box<dyn Plugin>>,

    plugins_enabled: bool,

    // Amount of time (in seconds) that passed from creation of the engine.
    elapsed_time: f32,

    /// A special container that is able to create nodes by their type UUID. Use a copy of this
    /// value whenever you need it as a parameter in other parts of the engine.
    pub serialization_context: Arc<SerializationContext>,

    script_processor: ScriptProcessor,
}

#[derive(Default)]
struct ScriptProcessor {
    wait_list: Vec<ResourceWaitContext>,
    scripted_scenes: FxHashSet<Handle<Scene>>,
}

impl ScriptProcessor {
    fn has_scripted_scene(&self, scene: Handle<Scene>) -> bool {
        self.scripted_scenes.contains(&scene)
    }

    fn register_scripted_scene(
        &mut self,
        scene: Handle<Scene>,
        scenes: &mut SceneContainer,
        resource_manager: &ResourceManager,
    ) {
        // Register the scene and ensure that it wasn't registered previously.
        let added = self.scripted_scenes.insert(scene);

        assert!(added);

        let graph = &mut scenes[scene].graph;

        // Spawn events for each node in the scene to force the engine to
        // initialize scripts.
        for (handle, _) in graph.pair_iter() {
            graph
                .script_message_sender
                .send(ScriptMessage::InitializeScript { handle })
                .unwrap();
        }

        self.wait_list
            .push(resource_manager.state().containers_mut().get_wait_context());
    }

    fn handle_scripts(
        &mut self,
        scenes: &mut SceneContainer,
        plugins: &mut Vec<Box<dyn Plugin>>,
        resource_manager: &ResourceManager,
        dt: f32,
        elapsed_time: f32,
    ) {
        self.wait_list
            .retain_mut(|context| !context.is_all_loaded());

        if !self.wait_list.is_empty() {
            return;
        }

        self.scripted_scenes
            .retain(|handle| scenes.is_valid_handle(*handle));

        'scene_loop: for &scene_handle in self.scripted_scenes.iter() {
            let scene = &mut scenes[scene_handle];

            // Disabled scenes should not update their scripts.
            if !scene.enabled {
                continue 'scene_loop;
            }

            // Fill in initial handles to nodes to update.
            let mut update_queue = VecDeque::new();
            for (handle, node) in scene.graph.pair_iter() {
                if let Some(script) = node.script.as_ref() {
                    if script.initialized && script.started {
                        update_queue.push_back(handle);
                    }
                }
            }

            // We'll gather all scripts queued for destruction and destroy them all at once at the
            // end of the frame.
            let mut destruction_queue = VecDeque::new();

            let max_iterations = 64;

            'update_loop: for update_loop_iteration in 0..max_iterations {
                let mut context = ScriptContext {
                    dt,
                    elapsed_time,
                    plugins,
                    handle: Default::default(),
                    scene,
                    resource_manager,
                };

                'init_loop: for init_loop_iteration in 0..max_iterations {
                    let mut start_queue = VecDeque::new();

                    // Process events first. `on_init` of a script can also create some other instances
                    // and these will be correctly initialized on current frame.
                    while let Ok(event) = context.scene.graph.script_message_receiver.try_recv() {
                        match event {
                            ScriptMessage::InitializeScript { handle } => {
                                context.handle = handle;

                                process_node(&mut context, &mut |script, context| {
                                    if !script.initialized {
                                        script.on_init(context);
                                        script.initialized = true;
                                    }

                                    // `on_start` must be called even if the script was initialized.
                                    start_queue.push_back(handle);
                                });
                            }
                            ScriptMessage::DestroyScript { handle, script } => {
                                // Destruction is delayed to the end of the frame.
                                destruction_queue.push_back((handle, script));
                            }
                        }
                    }

                    if start_queue.is_empty() {
                        // There is no more new nodes, we can safely leave the init loop.
                        break 'init_loop;
                    } else {
                        // Call `on_start` for every recently initialized node and go to next
                        // iteration of init loop. This is needed because `on_start` can spawn
                        // some other nodes that must be initialized before update.
                        while let Some(node) = start_queue.pop_front() {
                            context.handle = node;

                            process_node(&mut context, &mut |script, context| {
                                if !script.started {
                                    script.on_start(context);
                                    script.started = true;

                                    update_queue.push_back(node);
                                }
                            });
                        }
                    }

                    if init_loop_iteration == max_iterations - 1 {
                        Log::warn(
                            "Infinite init loop detected! Most likely some of \
                    your scripts causing infinite prefab instantiation!",
                        )
                    }
                }

                // Update all initialized and started scripts until there is something to initialize.
                if update_queue.is_empty() {
                    break 'update_loop;
                } else {
                    while let Some(handle) = update_queue.pop_front() {
                        context.handle = handle;

                        process_node(&mut context, &mut |script, context| {
                            script.on_update(context);
                        });
                    }
                }

                if update_loop_iteration == max_iterations - 1 {
                    Log::warn(
                        "Infinite update loop detected! Most likely some of \
                    your scripts causing infinite prefab instantiation!",
                    )
                }
            }

            // As the last step, destroy queued scripts.
            let mut context = ScriptDeinitContext {
                elapsed_time,
                plugins,
                resource_manager,
                scene,
                node_handle: Default::default(),
            };
            while let Some((handle, mut script)) = destruction_queue.pop_front() {
                context.node_handle = handle;

                // `on_deinit` could also spawn new nodes, but we won't take those into account on
                // this frame. They'll be correctly handled on next frame.
                script.on_deinit(&mut context);
            }
        }

        // Process scripts from destroyed scenes.
        for (handle, mut detached_scene) in scenes.destruction_list.drain(..) {
            if self.scripted_scenes.contains(&handle) {
                let mut context = ScriptDeinitContext {
                    elapsed_time,
                    plugins,
                    resource_manager,
                    scene: &mut detached_scene,
                    node_handle: Default::default(),
                };

                // Destroy every script instance from nodes that were still alive.
                for node_index in 0..context.scene.graph.capacity() {
                    context.node_handle = context.scene.graph.handle_from_index(node_index);

                    if let Some(mut script) = context
                        .scene
                        .graph
                        .try_get_mut(context.node_handle)
                        .and_then(|node| node.script.take())
                    {
                        // A script could not be initialized in case if we added a scene, and then immediately
                        // removed it. Calling `on_deinit` in this case would be a violation of API contract.
                        if script.initialized {
                            script.on_deinit(&mut context)
                        }
                    }
                }
            }
        }
    }
}

struct ResourceGraphVertex {
    resource: Model,
    children: Vec<ResourceGraphVertex>,
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
        }
    }

    pub fn resolve(&self) {
        Log::info(format!(
            "Resolving {} resource from dependency graph...",
            self.resource.state().path().display()
        ));

        // Wait until resource is fully loaded, then resolve.
        if block_on(self.resource.clone()).is_ok() {
            self.resource.data_ref().get_scene_mut().resolve();

            for child in self.children.iter() {
                child.resolve();
            }
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

fn process_node<T>(context: &mut ScriptContext, func: &mut T)
where
    T: FnMut(&mut Script, &mut ScriptContext),
{
    // Take a script from node. We're temporarily taking ownership over script
    // instance.
    let mut script = match context.scene.graph.try_get_mut(context.handle) {
        Some(node) => {
            if !node.is_globally_enabled() {
                return;
            }

            if let Some(script) = node.script.take() {
                script
            } else {
                // No script.
                return;
            }
        }
        None => {
            // Invalid handle.
            return;
        }
    };

    func(&mut script, context);

    // Put the script back to the node. We must do a checked borrow, because it is possible
    // that the node is already destroyed by script logic.
    if let Some(node) = context.scene.graph.try_get_mut(context.handle) {
        node.script = Some(script);
    }
}

pub(crate) fn process_scripts<T>(
    scene: &mut Scene,
    plugins: &mut [Box<dyn Plugin>],
    resource_manager: &ResourceManager,
    dt: f32,
    elapsed_time: f32,
    mut func: T,
) where
    T: FnMut(&mut Script, &mut ScriptContext),
{
    let mut context = ScriptContext {
        dt,
        elapsed_time,
        plugins,
        handle: Default::default(),
        scene,
        resource_manager,
    };

    for node_index in 0..context.scene.graph.capacity() {
        context.handle = context.scene.graph.handle_from_index(node_index);

        process_node(&mut context, &mut func);
    }
}

macro_rules! get_window {
    ($self:ident) => {{
        #[cfg(not(target_arch = "wasm32"))]
        {
            $self.context.window()
        }
        #[cfg(target_arch = "wasm32")]
        {
            &$self.window
        }
    }};
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
            user_interface: UserInterface::new(Vector2::new(client_size.x, client_size.y)),
            ui_time: Default::default(),
            #[cfg(not(target_arch = "wasm32"))]
            context,
            #[cfg(target_arch = "wasm32")]
            window,
            plugins: Default::default(),
            serialization_context: node_constructors,
            script_processor: Default::default(),
            plugins_enabled: false,
            plugin_constructors: Default::default(),
            elapsed_time: 0.0,
        })
    }

    /// Adjust size of the frame to be rendered. Must be called after the window size changes.
    /// Will update the renderer and GL context frame size.
    pub fn set_frame_size(&mut self, new_size: (u32, u32)) -> Result<(), FrameworkError> {
        self.renderer.set_frame_size(new_size)?;

        #[cfg(not(target_arch = "wasm32"))]
        self.context.resize(new_size.into());

        Ok(())
    }

    /// Amount of time (in seconds) that passed from creation of the engine. Keep in mind, that
    /// this value is **not** guaranteed to match real time. A user can change delta time with
    /// which the engine "ticks" and this delta time affects elapsed time.
    pub fn elapsed_time(&self) -> f32 {
        self.elapsed_time
    }

    /// Returns reference to main window. Could be useful to set fullscreen mode, change
    /// size of window, its title, etc.
    #[inline]
    pub fn get_window(&self) -> &Window {
        get_window!(self)
    }

    /// Performs single update tick with given time delta. Engine internally will perform update
    /// of all scenes, sub-systems, user interface, etc. Must be called in order to get engine
    /// functioning.
    ///
    /// ## Parameters
    ///
    /// `lag` - is a reference to time accumulator, that holds remaining amount of time that should be used
    /// to update a plugin. A caller splits `lag` into multiple sub-steps using `dt` and thus stabilizes
    /// update rate. The main use of this variable, is to be able to reset `lag` when you doing some heavy
    /// calculations in a your game loop (i.e. loading a new level) so the engine won't try to "catch up" with
    /// all the time that was spent in heavy calculation. The engine does **not** use this variable itself,
    /// but the plugins attach may use it, that's why you need to provide it. If you don't use plugins, then
    /// put `&mut 0.0` here.
    pub fn update(&mut self, dt: f32, control_flow: &mut ControlFlow, lag: &mut f32) {
        self.pre_update(dt, control_flow, lag);
        self.post_update(dt);
    }

    /// Performs pre update for the engine.
    ///
    /// Normally, this is called from `Engine::update()`.
    /// You should only call this manually if you don't use that method.
    ///
    /// ## Parameters
    ///
    /// `lag` - is a reference to time accumulator, that holds remaining amount of time that should be used
    /// to update a plugin. A caller splits `lag` into multiple sub-steps using `dt` and thus stabilizes
    /// update rate. The main use of this variable, is to be able to reset `lag` when you doing some heavy
    /// calculations in a your game loop (i.e. loading a new level) so the engine won't try to "catch up" with
    /// all the time that was spent in heavy calculation. The engine does **not** use this variable itself,
    /// but the plugins attach may use it, that's why you need to provide it. If you don't use plugins, then
    /// put `&mut 0.0` here.
    pub fn pre_update(&mut self, dt: f32, control_flow: &mut ControlFlow, lag: &mut f32) {
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

        self.update_plugins(dt, control_flow, lag);
        self.handle_scripts(dt);
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
        self.elapsed_time += dt;
    }

    /// Returns true if the scene is registered for script processing.
    pub fn has_scripted_scene(&self, scene: Handle<Scene>) -> bool {
        self.script_processor.has_scripted_scene(scene)
    }

    /// Registers a scene for script processing.
    pub fn register_scripted_scene(&mut self, scene: Handle<Scene>) {
        self.script_processor.register_scripted_scene(
            scene,
            &mut self.scenes,
            &self.resource_manager,
        )
    }

    fn handle_scripts(&mut self, dt: f32) {
        self.script_processor.handle_scripts(
            &mut self.scenes,
            &mut self.plugins,
            &self.resource_manager,
            dt,
            self.elapsed_time,
        );
    }

    fn update_plugins(&mut self, dt: f32, control_flow: &mut ControlFlow, lag: &mut f32) {
        if self.plugins_enabled {
            let mut context = PluginContext {
                scenes: &mut self.scenes,
                resource_manager: &self.resource_manager,
                renderer: &mut self.renderer,
                dt,
                lag,
                user_interface: &mut self.user_interface,
                serialization_context: &self.serialization_context,
                window: get_window!(self),
                sound_engine: SoundEngineHelper {
                    engine: &self.sound_engine,
                },
            };

            for plugin in self.plugins.iter_mut() {
                plugin.update(&mut context, control_flow);
            }

            while let Some(message) = self.user_interface.poll_message() {
                let mut context = PluginContext {
                    scenes: &mut self.scenes,
                    resource_manager: &self.resource_manager,
                    renderer: &mut self.renderer,
                    dt,
                    lag,
                    user_interface: &mut self.user_interface,
                    serialization_context: &self.serialization_context,
                    window: get_window!(self),
                    sound_engine: SoundEngineHelper {
                        engine: &self.sound_engine,
                    },
                };

                for plugin in self.plugins.iter_mut() {
                    plugin.on_ui_message(&mut context, &message, control_flow);
                }
            }
        }
    }

    /// Processes an OS event by every registered plugin.
    pub fn handle_os_event_by_plugins(
        &mut self,
        event: &Event<()>,
        dt: f32,
        control_flow: &mut ControlFlow,
        lag: &mut f32,
    ) {
        if self.plugins_enabled {
            for plugin in self.plugins.iter_mut() {
                plugin.on_os_event(
                    event,
                    PluginContext {
                        scenes: &mut self.scenes,
                        resource_manager: &self.resource_manager,
                        renderer: &mut self.renderer,
                        dt,
                        lag,
                        user_interface: &mut self.user_interface,
                        serialization_context: &self.serialization_context,
                        window: get_window!(self),
                        sound_engine: SoundEngineHelper {
                            engine: &self.sound_engine,
                        },
                    },
                    control_flow,
                );
            }
        }
    }

    /// Passes specified OS event to every script of the specified scene.
    ///
    /// # Important notes
    ///
    /// This method is intended to be used by the editor and game runner. If you're using the
    /// engine as a framework, then you should not call this method because you'll most likely
    /// do something wrong.
    pub(crate) fn handle_os_event_by_scripts(
        &mut self,
        event: &Event<()>,
        scene: Handle<Scene>,
        dt: f32,
    ) {
        let scene = &mut self.scenes[scene];
        if scene.enabled {
            process_scripts(
                scene,
                &mut self.plugins,
                &self.resource_manager,
                dt,
                self.elapsed_time,
                |script, context| {
                    if script.initialized {
                        script.on_os_event(event, context);
                    }
                },
            )
        }
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

                Log::info("Propagating changes to active scenes...");

                // Resolve all scenes.
                // TODO: This might be inefficient if there is bunch of scenes loaded,
                // however this seems to be very rare case so it should be ok.
                for scene in self.scenes.iter_mut() {
                    scene.resolve();
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

    /// Enables or disables registered plugins.
    pub(crate) fn enable_plugins(&mut self, override_scene: Handle<Scene>, enabled: bool) {
        if self.plugins_enabled != enabled {
            self.plugins_enabled = enabled;

            if self.plugins_enabled {
                // Create and initialize instances.
                for constructor in self.plugin_constructors.iter() {
                    self.plugins.push(constructor.create_instance(
                        override_scene,
                        PluginContext {
                            scenes: &mut self.scenes,
                            resource_manager: &self.resource_manager,
                            renderer: &mut self.renderer,
                            dt: 0.0,
                            lag: &mut 0.0,
                            user_interface: &mut self.user_interface,
                            serialization_context: &self.serialization_context,
                            window: get_window!(self),
                            sound_engine: SoundEngineHelper {
                                engine: &self.sound_engine,
                            },
                        },
                    ));
                }
            } else {
                self.handle_scripts(0.0);

                for mut plugin in self.plugins.drain(..) {
                    // Deinit plugin first.
                    plugin.on_deinit(PluginContext {
                        scenes: &mut self.scenes,
                        resource_manager: &self.resource_manager,
                        renderer: &mut self.renderer,
                        dt: 0.0,
                        lag: &mut 0.0,
                        user_interface: &mut self.user_interface,
                        serialization_context: &self.serialization_context,
                        window: get_window!(self),
                        sound_engine: SoundEngineHelper {
                            engine: &self.sound_engine,
                        },
                    });
                }
            }
        }
    }

    /// Adds new plugin plugin constructor.
    pub fn add_plugin_constructor<P>(&mut self, constructor: P)
    where
        P: PluginConstructor + 'static,
    {
        constructor.register(PluginRegistrationContext {
            serialization_context: &self.serialization_context,
        });

        self.plugin_constructors.push(Box::new(constructor));
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // Destroy all scenes first and correctly destroy all script instances.
        // This will ensure that any `on_destroy` logic will be executed before
        // engine destroyed.
        let scenes = self
            .scenes
            .pair_iter()
            .map(|(h, _)| h)
            .collect::<Vec<Handle<Scene>>>();

        for handle in scenes {
            self.scenes.remove(handle);
        }

        // Finally disable plugins.
        self.enable_plugins(Default::default(), false);
    }
}

#[cfg(test)]
mod test {
    use crate::{
        core::{pool::Handle, reflect::prelude::*, uuid::Uuid, visitor::prelude::*},
        engine::{resource_manager::ResourceManager, ScriptProcessor},
        impl_component_provider,
        scene::{base::BaseBuilder, node::Node, pivot::PivotBuilder, Scene, SceneContainer},
        script::{Script, ScriptContext, ScriptDeinitContext, ScriptTrait},
    };
    use std::sync::mpsc::{self, Sender, TryRecvError};

    #[derive(PartialEq, Eq, Clone, Debug)]
    enum Event {
        Initialized(Handle<Node>),
        Started(Handle<Node>),
        Updated(Handle<Node>),
        Destroyed(Handle<Node>),
    }

    #[derive(Debug, Clone, Reflect, Visit)]
    struct MyScript {
        #[reflect(hidden)]
        #[visit(skip)]
        sender: Sender<Event>,
        spawned: bool,
    }

    impl_component_provider!(MyScript);

    impl ScriptTrait for MyScript {
        fn on_init(&mut self, ctx: &mut ScriptContext) {
            self.sender.send(Event::Initialized(ctx.handle)).unwrap();

            // Spawn new entity with script.
            let handle =
                PivotBuilder::new(BaseBuilder::new().with_script(Script::new(MySubScript {
                    sender: self.sender.clone(),
                })))
                .build(&mut ctx.scene.graph);
            assert_eq!(handle, Handle::new(2, 1));
        }

        fn on_start(&mut self, ctx: &mut ScriptContext) {
            self.sender.send(Event::Started(ctx.handle)).unwrap();

            // Spawn new entity with script.
            let handle =
                PivotBuilder::new(BaseBuilder::new().with_script(Script::new(MySubScript {
                    sender: self.sender.clone(),
                })))
                .build(&mut ctx.scene.graph);
            assert_eq!(handle, Handle::new(3, 1));
        }

        fn on_deinit(&mut self, ctx: &mut ScriptDeinitContext) {
            self.sender.send(Event::Destroyed(ctx.node_handle)).unwrap();
        }

        fn on_update(&mut self, ctx: &mut ScriptContext) {
            self.sender.send(Event::Updated(ctx.handle)).unwrap();

            if !self.spawned {
                // Spawn new entity with script.
                PivotBuilder::new(BaseBuilder::new().with_script(Script::new(MySubScript {
                    sender: self.sender.clone(),
                })))
                .build(&mut ctx.scene.graph);

                self.spawned = true;
            }
        }

        fn id(&self) -> Uuid {
            Uuid::new_v4()
        }
    }

    #[derive(Debug, Clone, Reflect, Visit)]
    struct MySubScript {
        #[reflect(hidden)]
        #[visit(skip)]
        sender: Sender<Event>,
    }

    impl_component_provider!(MySubScript);

    impl ScriptTrait for MySubScript {
        fn on_init(&mut self, ctx: &mut ScriptContext) {
            self.sender.send(Event::Initialized(ctx.handle)).unwrap();
        }

        fn on_start(&mut self, ctx: &mut ScriptContext) {
            self.sender.send(Event::Started(ctx.handle)).unwrap();
        }

        fn on_deinit(&mut self, ctx: &mut ScriptDeinitContext) {
            self.sender.send(Event::Destroyed(ctx.node_handle)).unwrap();
        }

        fn on_update(&mut self, ctx: &mut ScriptContext) {
            self.sender.send(Event::Updated(ctx.handle)).unwrap();
        }

        fn id(&self) -> Uuid {
            Uuid::new_v4()
        }
    }

    #[test]
    fn test_order() {
        let resource_manager = ResourceManager::new(Default::default());
        let mut scene = Scene::new();

        let (tx, rx) = mpsc::channel();

        let node_handle =
            PivotBuilder::new(BaseBuilder::new().with_script(Script::new(MyScript {
                sender: tx,
                spawned: false,
            })))
            .build(&mut scene.graph);
        assert_eq!(node_handle, Handle::new(1, 1));

        let mut scene_container = SceneContainer::new(Default::default());

        let scene_handle = scene_container.add(scene);

        let mut script_processor = ScriptProcessor::default();

        script_processor.register_scripted_scene(
            scene_handle,
            &mut scene_container,
            &resource_manager,
        );

        let handle_on_init = Handle::new(2, 1);
        let handle_on_start = Handle::new(3, 1);
        let handle_on_update1 = Handle::new(4, 1);

        for iteration in 0..3 {
            script_processor.handle_scripts(
                &mut scene_container,
                &mut Default::default(),
                &resource_manager,
                0.0,
                0.0,
            );

            match iteration {
                0 => {
                    assert_eq!(rx.try_recv(), Ok(Event::Initialized(node_handle)));
                    assert_eq!(rx.try_recv(), Ok(Event::Initialized(handle_on_init)));

                    assert_eq!(rx.try_recv(), Ok(Event::Started(node_handle)));
                    assert_eq!(rx.try_recv(), Ok(Event::Started(handle_on_init)));

                    assert_eq!(rx.try_recv(), Ok(Event::Initialized(handle_on_start)));
                    assert_eq!(rx.try_recv(), Ok(Event::Started(handle_on_start)));

                    assert_eq!(rx.try_recv(), Ok(Event::Updated(node_handle)));
                    assert_eq!(rx.try_recv(), Ok(Event::Updated(handle_on_init)));
                    assert_eq!(rx.try_recv(), Ok(Event::Updated(handle_on_start)));

                    assert_eq!(rx.try_recv(), Ok(Event::Initialized(handle_on_update1)));
                    assert_eq!(rx.try_recv(), Ok(Event::Started(handle_on_update1)));

                    assert_eq!(rx.try_recv(), Ok(Event::Updated(handle_on_update1)));
                }
                1 => {
                    assert_eq!(rx.try_recv(), Ok(Event::Updated(node_handle)));
                    assert_eq!(rx.try_recv(), Ok(Event::Updated(handle_on_init)));
                    assert_eq!(rx.try_recv(), Ok(Event::Updated(handle_on_start)));
                    assert_eq!(rx.try_recv(), Ok(Event::Updated(handle_on_update1)));
                    assert_eq!(rx.try_recv(), Err(TryRecvError::Empty));

                    // Now destroy every node with script, next iteration should correctly destroy attached scripts.
                    let scene = &mut scene_container[scene_handle];
                    scene.remove_node(node_handle);
                    scene.remove_node(handle_on_init);
                    scene.remove_node(handle_on_start);
                    scene.remove_node(handle_on_update1);
                }
                2 => {
                    assert_eq!(rx.try_recv(), Ok(Event::Destroyed(node_handle)));
                    assert_eq!(rx.try_recv(), Ok(Event::Destroyed(handle_on_init)));
                    assert_eq!(rx.try_recv(), Ok(Event::Destroyed(handle_on_start)));
                    assert_eq!(rx.try_recv(), Ok(Event::Destroyed(handle_on_update1)));

                    // Every instance holding sender died, so receiver is disconnected from sender.
                    assert_eq!(rx.try_recv(), Err(TryRecvError::Disconnected));
                }
                _ => (),
            }
        }
    }
}
