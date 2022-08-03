#![warn(missing_docs)]

//! Script is used to add custom logic to scene nodes. See [ScriptTrait] for more info.

use crate::{
    core::{
        inspect::{Inspect, PropertyInfo},
        pool::Handle,
        reflect::{Reflect, ReflectArray, ReflectList},
        uuid::Uuid,
        visitor::{Visit, VisitResult, Visitor},
    },
    engine::resource_manager::ResourceManager,
    event::Event,
    plugin::Plugin,
    scene::{graph::map::NodeHandleMap, node::Node, Scene},
    utils::component::ComponentProvider,
};
use std::{
    any::{Any, TypeId},
    fmt::Debug,
    ops::{Deref, DerefMut},
};

pub mod constructor;

/// Base script trait is used to automatically implement some trait to reduce amount of boilerplate code.
pub trait BaseScript: Visit + Inspect + Reflect + Send + Debug + 'static {
    /// Creates exact copy of the script.
    fn clone_box(&self) -> Box<dyn ScriptTrait>;
}

impl<T> BaseScript for T
where
    T: Clone + ScriptTrait + Any,
{
    fn clone_box(&self) -> Box<dyn ScriptTrait> {
        Box::new(self.clone())
    }
}

/// A set of data, that provides contextual information for script methods.
pub struct ScriptContext<'a, 'b> {
    /// Amount of time that passed from last call. It has valid values only when called from `on_update`.
    pub dt: f32,

    /// A reference to the plugin which the script instance belongs to. You can use it to access plugin data
    /// inside script methods. For example you can store some "global" data in the plugin - for example a
    /// controls configuration, some entity managers and so on.
    pub plugin: &'a mut dyn Plugin,

    /// Handle of a node to which the script instance belongs to. To access the node itself use `scene` field:
    ///
    /// ```rust
    /// # use fyrox::script::ScriptContext;
    /// # fn foo(context: ScriptContext) {
    /// let node_mut = &mut context.scene.graph[context.handle];
    /// # }
    /// ```
    pub handle: Handle<Node>,

    /// A reference to a scene the script instance belongs to. You have full mutable access to scene content
    /// in most of the script methods.
    pub scene: &'b mut Scene,

    /// A reference to resource manager, use it to load resources.
    pub resource_manager: &'a ResourceManager,
}

/// A set of data that will be passed to a script instance just before its destruction.
pub struct ScriptDeinitContext<'a, 'b> {
    /// A reference to the plugin which the script instance belongs to. You can use it to access plugin data
    /// inside script methods. For example you can store some "global" data in the plugin - for example a
    /// controls configuration, some entity managers and so on.
    pub plugin: &'a mut dyn Plugin,

    /// A reference to resource manager, use it to load resources.
    pub resource_manager: &'a ResourceManager,

    /// A reference to a scene the script instance was belonging to. You have full mutable access to scene content
    /// in most of the script methods.
    pub scene: &'b mut Scene,

    /// Handle to a parent scene node. Use it with caution because parent node could be deleted already and
    /// any unchecked borrowing using the handle will cause panic!
    pub node_handle: Handle<Node>,
}

/// Script is a set predefined methods that are called on various stages by the engine. It is used to add
/// custom behaviour to game entities.
pub trait ScriptTrait: BaseScript + ComponentProvider {
    /// The method is called when the script wasn't initialized yet. It is guaranteed to be called once,
    /// and before any other methods of the script.
    ///
    /// # Editor-specific information
    ///
    /// In the editor, the method will be called on entering the play mode.
    fn on_init(&mut self, #[allow(unused_variables)] context: ScriptContext) {}

    /// The method is called when the script is about to be destroyed. It is guaranteed to be called last.
    ///
    /// # Editor-specific information
    ///
    /// In the editor, the method will be called before leaving the play mode.
    fn on_deinit(&mut self, #[allow(unused_variables)] context: ScriptDeinitContext) {}

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
    fn remap_handles(&mut self, #[allow(unused_variables)] old_new_mapping: &NodeHandleMap) {}

    /// Allows you to restore resources after deserialization.
    ///
    /// # Motivation
    ///
    /// Some scripts may store resources "handles" (for example a texture or a 3d model), when the
    /// handle is saved, only path to resource is saved. When you loading a save, you must ask resource
    /// manager to restore handles.
    fn restore_resources(&mut self, #[allow(unused_variables)] resource_manager: ResourceManager) {}

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
    ///     core::reflect::Reflect,
    ///     core::uuid::Uuid,
    ///     script::ScriptTrait,
    ///     core::uuid::uuid,
    ///     impl_component_provider,
    /// };
    ///
    /// #[derive(Inspect, Reflect, Visit, Debug, Clone)]
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
    /// impl_component_provider!(MyScript);
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

/// A wrapper for actual script instance internals, it used by the engine.
#[derive(Debug)]
pub struct Script(pub Box<dyn ScriptTrait>);

impl Reflect for Script {
    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self.0.into_any()
    }

    fn as_any(&self) -> &dyn Any {
        self.0.deref().as_any()
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self.0.deref_mut().as_any_mut()
    }

    fn as_reflect(&self) -> &dyn Reflect {
        self.0.deref().as_reflect()
    }

    fn as_reflect_mut(&mut self) -> &mut dyn Reflect {
        self.0.deref_mut().as_reflect_mut()
    }

    fn set(&mut self, value: Box<dyn Reflect>) -> Result<Box<dyn Reflect>, Box<dyn Reflect>> {
        self.0.deref_mut().set(value)
    }

    fn field(&self, name: &str) -> Option<&dyn Reflect> {
        self.0.deref().field(name)
    }

    fn field_mut(&mut self, name: &str) -> Option<&mut dyn Reflect> {
        self.0.deref_mut().field_mut(name)
    }

    fn as_array(&self) -> Option<&dyn ReflectArray> {
        self.0.deref().as_array()
    }

    fn as_array_mut(&mut self) -> Option<&mut dyn ReflectArray> {
        self.0.deref_mut().as_array_mut()
    }

    fn as_list(&self) -> Option<&dyn ReflectList> {
        self.0.deref().as_list()
    }

    fn as_list_mut(&mut self) -> Option<&mut dyn ReflectList> {
        self.0.deref_mut().as_list_mut()
    }
}

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
    /// Creates new script wrapper using given script instance.
    pub fn new<T: ScriptTrait>(script_object: T) -> Self {
        Self(Box::new(script_object))
    }

    /// Performs downcasting to a particular type.
    pub fn cast<T: ScriptTrait>(&self) -> Option<&T> {
        self.0.as_any().downcast_ref::<T>()
    }

    /// Performs downcasting to a particular type.
    pub fn cast_mut<T: ScriptTrait>(&mut self) -> Option<&mut T> {
        self.0.as_any_mut().downcast_mut::<T>()
    }

    /// Tries to borrow a component of given type.
    pub fn query_component_ref<T: Any>(&self) -> Option<&T> {
        self.0
            .query_component_ref(TypeId::of::<T>())
            .and_then(|c| c.downcast_ref())
    }

    /// Tries to borrow a component of given type.
    pub fn query_component_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.0
            .query_component_mut(TypeId::of::<T>())
            .and_then(|c| c.downcast_mut())
    }
}

/// A helper macro that allows you to handle object's property changed message. Such messages may come
/// from editor's Inspector. It handles Object variant of the message.
///
/// # Examples
///
/// ```rust
/// use fyrox::handle_object_property_changed;
/// use fyrox::gui::inspector::{PropertyChanged, FieldKind};
/// use fyrox::core::inspect::{Inspect, PropertyInfo};
/// use fyrox::core::reflect::Reflect;
///
/// #[derive(Inspect, Reflect)]
/// struct Foo {
///     bar: String,
///     baz: u32
/// }
///
/// impl Foo {
///     fn on_property_changed(&mut self, args: &PropertyChanged) -> bool {
///          handle_object_property_changed!(self, args,
///             Self::BAR => bar,
///             Self::BAZ => baz
///          )
///     }
/// }
/// ```
///
/// This will apply changes to the respective properties in a few lines of code. The main reason of this
/// macro to exist is to reduce amount of boilerplate code.
#[macro_export]
macro_rules! handle_object_property_changed {
    ($self:expr, $args:expr, $($prop:path => $field:tt),*) => {
        match $args.value {
            $crate::gui::inspector::FieldKind::Object(ref value) => {
                match $args.name.as_ref() {
                    $($prop => {
                        $self.$field = value.cast_clone().unwrap();
                        true
                    })*
                    _ => false,
                }
            }
            _ => false
        }
    }
}

/// A helper macro that allows you to handle object's property changed message. Such messages may come
/// from editor's Inspector. It handles CollectionChanged variant of the message. The type of the collection
/// item **must** have a method called `on_property_changed` - you could use newtype for that (see examples).
///
/// # Examples
///
/// ```rust
/// use fyrox::{handle_collection_property_changed, handle_object_property_changed};
/// use fyrox::gui::inspector::{PropertyChanged, CollectionChanged, FieldKind};
/// use fyrox::core::inspect::{Inspect, PropertyInfo};
/// use fyrox::core::reflect::Reflect;
///
/// // Wrap parameter in a newtype to implement `on_property_changed` method.
/// #[derive(Inspect, Reflect, Debug, Default)]
/// struct Name(String);
///
/// impl Name {
///     fn on_property_changed(&mut self, args: &PropertyChanged) -> bool {
///         handle_object_property_changed!(self, args, Self::F_0 => 0)
///     }
/// }
///
/// #[derive(Inspect, Reflect)]
/// struct Foo {
///     // Collections could be any type that has `push`, `remove(index)`, `impl IndexMut`
///     names: Vec<Name>,
///     other_names: Vec<Name>,
/// }
///
/// impl Foo {
///     fn on_property_changed(&mut self, args: &PropertyChanged) -> bool {
///         handle_collection_property_changed!(self, args,
///             Self::NAMES => names,
///             Self::OTHER_NAMES => other_names
///         )
///     }
/// }
/// ```
///
/// This will apply changes to the respective properties in a few lines of code. The main reason of this
/// macro to exist is to reduce amount of boilerplate code.
#[macro_export]
macro_rules! handle_collection_property_changed {
    ($self:expr, $args:expr, $($prop:path => $field:ident),*) => {
        match $args.value {
            FieldKind::Collection(ref collection) => match $args.name.as_ref() {
                 $($prop => {
                     match **collection {
                        $crate::gui::inspector::CollectionChanged::Add(_) => {
                            $self.$field.push(Default::default());
                            true
                        }
                        $crate::gui::inspector::CollectionChanged::Remove(i) => {
                            $self.$field.remove(i);
                            true
                        }
                        $crate::gui::inspector::CollectionChanged::ItemChanged {
                            index,
                            ref property,
                        } => $self.$field[index].on_property_changed(property),
                    }
                })*,
                _ => false
            },
            _ => false
        }
    }
}
