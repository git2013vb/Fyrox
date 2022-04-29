use crate::engine::resource_manager::ResourceManager;
use crate::{
    core::{
        inspect::{Inspect, PropertyInfo},
        pool::Handle,
        uuid::Uuid,
        visitor::{Visit, VisitResult, Visitor},
    },
    event::Event,
    gui::inspector::PropertyChanged,
    plugin::Plugin,
    scene::{node::Node, Scene},
};
use fxhash::FxHashMap;
use std::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

pub mod constructor;

pub trait BaseScript: Visit + Inspect + Send + Debug + 'static {
    fn clone_box(&self) -> Box<dyn ScriptTrait>;
}

impl<T> BaseScript for T
where
    T: Clone + ScriptTrait,
{
    fn clone_box(&self) -> Box<dyn ScriptTrait> {
        Box::new(self.clone())
    }
}

pub struct ScriptContext<'a, 'b, 'c> {
    pub dt: f32,
    pub plugin: &'a mut dyn Plugin,
    pub node: &'b mut Node,
    pub handle: Handle<Node>,
    pub scene: &'c mut Scene,
    pub resource_manager: &'a ResourceManager,
}

pub trait ScriptTrait: BaseScript {
    /// Mutates the state of the script according to the [`PropertyChanged`] info. It is invoked
    /// from the editor when user changes property of the script from the inspector.
    ///
    /// # Motivation
    ///
    /// Why the editor cannot mutate variable for me so I don't need to do it by hand? The answer
    /// is pretty simple - UI system does not know anything about your object, it uses its own data
    /// model, the only thing it could do is to indicate that some value was changed so you can
    /// react to it.
    ///
    /// # Return value
    ///
    /// The return value of the method indicates whether the change was applied to the script data
    /// or not. If nothing changed (the return value was `false`)  the editor will give you a
    /// diagnostic message that the change in Inspector had no effect and probably a property handler
    /// is missing.
    ///
    /// # Important notes
    ///
    /// Works only in **editor mode**.
    ///
    /// # Example
    ///
    /// ```rust
    /// use fyrox::gui::inspector::{PropertyChanged, FieldKind};
    /// use fyrox::script::ScriptTrait;
    /// use fyrox::core::uuid::Uuid;
    /// use fyrox::core::inspect::{Inspect, PropertyInfo};
    /// use fyrox::core::visitor::prelude::*;
    ///
    /// #[derive(Inspect, Visit, Debug, Clone)]
    /// struct MyScript {
    ///     foo: f32,
    ///     bar: String,
    /// }
    ///
    /// // Some functions are intentionally omitted.
    ///
    /// impl ScriptTrait for MyScript {
    ///     fn on_property_changed(&mut self, args: &PropertyChanged) -> bool {
    ///         if let FieldKind::Object(ref value) = args.value {
    ///             return match args.name.as_ref() {
    ///                 Self::FOO => value.try_override(&mut self.foo),
    ///                 Self::BAR => value.try_override(&mut self.bar),
    ///                 _ => false
    ///             }
    ///         }
    ///
    ///         // Nothing changed, in this case the editor will give you a diagnostic message
    ///         // that the change in Inspector had no effect and probably property handler is
    ///         // missing.
    ///         false
    ///     }
    ///
    ///     // ...
    ///    # fn id(&self) -> Uuid {
    ///    #     todo!()
    ///    # }
    ///
    ///    # fn plugin_uuid(&self) -> Uuid {
    ///    #     todo!()
    ///    # }
    /// }
    /// ```
    fn on_property_changed(&mut self, #[allow(unused_variables)] args: &PropertyChanged) -> bool {
        false
    }

    /// Called on parent scene initialization. It is guaranteed to be called once, and before any
    /// other method of the script.
    ///
    /// # Editor-specific infomation
    ///
    /// In the editor, the method will be called on entering the play mode.
    fn on_init(&mut self, #[allow(unused_variables)] context: ScriptContext) {}

    /// Called when there is an event from the OS. The method allows you to "listen" for events
    /// coming from the main window of your game (or the editor if the game running inside the
    /// editor.
    ///
    /// # Editor-specific information
    ///
    /// When the game running inside the editor, every event related to position/size changes will
    /// be modified to have position/size of the preview frame of the editor, not the main window.
    /// For end user this means that the game will function as if it was run in standalone mode.
    fn on_os_event(
        &mut self,
        #[allow(unused_variables)] event: &Event<()>,
        #[allow(unused_variables)] context: ScriptContext,
    ) {
    }

    /// Performs a single update tick of the script. The method may be called multiple times per
    /// frame, but it is guaranteed that the rate of call is stable and usually it will be called
    /// 60 times per second (this may change in future releases).
    ///
    /// # Editor-specific information
    ///
    /// Does not work in editor mode, works only in play mode.
    fn on_update(&mut self, #[allow(unused_variables)] context: ScriptContext) {}

    /// Called right after the parent node was copied, giving you the ability to remap handles to
    /// nodes stored inside of your script.
    ///
    /// # Motivation
    ///
    /// Imagine that you have a character controller script that contains handles to some other
    /// nodes in the scene, for example a collider. When you copy the node with the script, you
    /// want the copy to contain references to respective copies, not the original objects.
    /// The method allows you to do exactly this.
    fn remap_handles(
        &mut self,
        #[allow(unused_variables)] old_new_mapping: &FxHashMap<Handle<Node>, Handle<Node>>,
    ) {
    }

    /// Script instance type UUID. The value will be used for serialization, to write type
    /// identifier to a data source so the engine can restore the script from data source.
    ///
    /// # Important notes
    ///
    /// Do **not** use [`Uuid::new_v4`] or any other [`Uuid`] methods that generates ids, ids
    /// generated using these methods are **random** and are not suitable for serialization!
    ///
    /// # Example
    ///
    /// All you need to do in the method is to return `Self::type_uuid`.
    ///
    /// ```rust
    /// use std::str::FromStr;
    /// use fyrox::{
    ///     scene::node::TypeUuidProvider,
    ///     core::visitor::prelude::*,
    ///     core::inspect::{Inspect, PropertyInfo},
    ///     core::uuid::Uuid,
    ///     script::ScriptTrait,
    ///     core::uuid::uuid
    /// };
    ///
    /// #[derive(Inspect, Visit, Debug, Clone)]
    /// struct MyScript { }
    ///
    /// // Implement TypeUuidProvider trait that will return type uuid of the type.
    /// // Every script must implement the trait so the script can be registered in
    /// // serialization context of the engine.
    /// impl TypeUuidProvider for MyScript {
    ///     fn type_uuid() -> Uuid {
    ///         // Use https://www.uuidgenerator.net/ to generate new UUID.
    ///         uuid!("4cfbe65e-a2c1-474f-b123-57516d80b1f8")
    ///     }
    /// }
    ///
    /// impl ScriptTrait for MyScript {
    ///     fn id(&self) -> Uuid {
    ///         Self::type_uuid()
    ///     }
    ///
    ///    # fn plugin_uuid(&self) -> Uuid {
    ///    #     todo!()
    ///    # }
    /// }
    /// ```
    fn id(&self) -> Uuid;

    /// Returns parent plugin UUID. It is used to find respective plugin when processing scripts.
    /// The engine makes an attempt to find a plugin by comparing type uuids and if one found,
    /// it is passed on ScriptContext.
    fn plugin_uuid(&self) -> Uuid;
}

#[derive(Debug)]
pub struct Script(pub Box<dyn ScriptTrait>);

impl Deref for Script {
    type Target = dyn ScriptTrait;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl DerefMut for Script {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut *self.0
    }
}

impl Inspect for Script {
    fn properties(&self) -> Vec<PropertyInfo<'_>> {
        self.0.properties()
    }
}

impl Visit for Script {
    fn visit(&mut self, name: &str, visitor: &mut Visitor) -> VisitResult {
        self.0.visit(name, visitor)
    }
}

impl Clone for Script {
    fn clone(&self) -> Self {
        Self(self.0.clone_box())
    }
}

impl Script {
    pub fn new<T: ScriptTrait>(script_object: T) -> Self {
        Self(Box::new(script_object))
    }
}
