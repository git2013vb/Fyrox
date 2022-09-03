use crate::{
    inspector::editors::{
        handle::HandlePropertyEditorDefinition, material::MaterialPropertyEditorDefinition,
        resource::ResourceFieldPropertyEditorDefinition, script::ScriptPropertyEditorDefinition,
        texture::TexturePropertyEditorDefinition,
    },
    Message,
};
use fyrox::scene::node::NodeHandle;
use fyrox::{
    animation::{
        machine::MachineInstantiationError,
        spritesheet::{self, FrameBounds, SpriteSheetAnimation},
    },
    core::{
        futures::executor::block_on,
        parking_lot::Mutex,
        pool::{ErasedHandle, Handle},
    },
    gui::inspector::editors::{
        array::ArrayPropertyEditorDefinition, bit::BitFieldPropertyEditorDefinition,
        collection::VecCollectionPropertyEditorDefinition,
        enumeration::EnumPropertyEditorDefinition,
        inspectable::InspectablePropertyEditorDefinition, PropertyEditorDefinitionContainer,
    },
    material::shader::{Shader, ShaderError, ShaderState},
    resource::{
        absm::{AbsmResource, AbsmResourceState},
        curve::{CurveResource, CurveResourceError, CurveResourceState},
        model::{MaterialSearchOptions, Model, ModelData, ModelLoadError},
        texture::{
            CompressionOptions, TextureMagnificationFilter, TextureMinificationFilter,
            TextureWrapMode,
        },
    },
    scene::{
        base::{
            Base, LevelOfDetail, LodControlledObject, LodGroup, Mobility, Property, PropertyValue,
        },
        camera::{
            ColorGradingLut, Exposure, OrthographicProjection, PerspectiveProjection, Projection,
            SkyBox,
        },
        collider::{
            BallShape, BitMask, CapsuleShape, ColliderShape, ConeShape, ConvexPolyhedronShape,
            CuboidShape, CylinderShape, GeometrySource, HeightfieldShape, InteractionGroups,
            SegmentShape, TriangleShape, TrimeshShape,
        },
        dim2,
        graph::physics::CoefficientCombineRule,
        joint::*,
        light::{
            directional::{CsmOptions, FrustumSplitOptions},
            BaseLight,
        },
        mesh::{surface::Surface, RenderPath},
        node::Node,
        particle_system::emitter::{
            base::BaseEmitter, cuboid::CuboidEmitter, cylinder::CylinderEmitter,
            sphere::SphereEmitter, Emitter,
        },
        particle_system::EmitterWrapper,
        rigidbody::RigidBodyType,
        sound::{
            self,
            effect::{BaseEffect, Effect, EffectInput, ReverbEffect},
            Biquad, DistanceModel, SoundBufferResource, SoundBufferResourceLoadError,
            SoundBufferState, Status,
        },
        terrain::Layer,
        transform::Transform,
    },
};
use std::{rc::Rc, sync::mpsc::Sender};

pub mod handle;
pub mod material;
pub mod resource;
pub mod script;
pub mod texture;

pub fn make_status_enum_editor_definition() -> EnumPropertyEditorDefinition<Status> {
    EnumPropertyEditorDefinition {
        variant_generator: |i| match i {
            0 => Status::Stopped,
            1 => Status::Playing,
            2 => Status::Paused,
            _ => unreachable!(),
        },
        index_generator: |v| *v as usize,
        names_generator: || {
            vec![
                "Stopped".to_string(),
                "Playing".to_string(),
                "Paused".to_string(),
            ]
        },
    }
}

pub fn make_property_editors_container(
    sender: Sender<Message>,
) -> PropertyEditorDefinitionContainer {
    let container = PropertyEditorDefinitionContainer::new();

    container.insert(TexturePropertyEditorDefinition);
    container.insert(MaterialPropertyEditorDefinition {
        sender: Mutex::new(sender.clone()),
    });
    container.insert(VecCollectionPropertyEditorDefinition::<Surface>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<Layer>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<EmitterWrapper>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<LevelOfDetail>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<ErasedHandle>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<Handle<Node>>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<Property>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<LodControlledObject>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<GeometrySource>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<EffectInput>::new());
    container.insert(make_status_enum_editor_definition());
    container.insert(EnumPropertyEditorDefinition::<f32>::new_optional());
    container.insert(EnumPropertyEditorDefinition::<u32>::new_optional());
    container.insert(EnumPropertyEditorDefinition::<LodGroup>::new_optional());
    container.insert(EnumPropertyEditorDefinition::<spritesheet::Status>::new());
    container.insert(InspectablePropertyEditorDefinition::<NodeHandle>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<NodeHandle>::new());
    container.insert(InspectablePropertyEditorDefinition::<LodGroup>::new());
    container.insert(InspectablePropertyEditorDefinition::<SpriteSheetAnimation>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<SpriteSheetAnimation>::new());
    container.insert(InspectablePropertyEditorDefinition::<FrameBounds>::new());
    container.insert(VecCollectionPropertyEditorDefinition::<FrameBounds>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        spritesheet::signal::Signal,
    >::new());
    container.insert(VecCollectionPropertyEditorDefinition::<
        spritesheet::signal::Signal,
    >::new());
    container.insert(ResourceFieldPropertyEditorDefinition::<
        Model,
        ModelData,
        ModelLoadError,
    >::new(Rc::new(|resource_manager, path| {
        block_on(resource_manager.request_model(path))
    })));
    container.insert(ResourceFieldPropertyEditorDefinition::<
        SoundBufferResource,
        SoundBufferState,
        SoundBufferResourceLoadError,
    >::new(Rc::new(|resource_manager, path| {
        block_on(resource_manager.request_sound_buffer(path))
    })));
    container.insert(ResourceFieldPropertyEditorDefinition::<
        AbsmResource,
        AbsmResourceState,
        MachineInstantiationError,
    >::new(Rc::new(|resource_manager, path| {
        block_on(resource_manager.request_absm(path))
    })));
    container.insert(ResourceFieldPropertyEditorDefinition::<
        CurveResource,
        CurveResourceState,
        CurveResourceError,
    >::new(Rc::new(|resource_manager, path| {
        block_on(resource_manager.request_curve(path))
    })));
    container.insert(ResourceFieldPropertyEditorDefinition::<
        Shader,
        ShaderState,
        ShaderError,
    >::new(Rc::new(|resource_manager, path| {
        block_on(resource_manager.request_shader(path))
    })));
    container.insert(InspectablePropertyEditorDefinition::<ColorGradingLut>::new());
    container.insert(InspectablePropertyEditorDefinition::<InteractionGroups>::new());
    container.insert(InspectablePropertyEditorDefinition::<ColliderShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<GeometrySource>::new());
    container.insert(EnumPropertyEditorDefinition::<JointParams>::new());
    container.insert(EnumPropertyEditorDefinition::<dim2::joint::JointParams>::new());
    container.insert(InspectablePropertyEditorDefinition::<BallJoint>::new());
    container.insert(InspectablePropertyEditorDefinition::<dim2::joint::BallJoint>::new());
    container.insert(InspectablePropertyEditorDefinition::<FixedJoint>::new());
    container.insert(InspectablePropertyEditorDefinition::<dim2::joint::FixedJoint>::new());
    container.insert(InspectablePropertyEditorDefinition::<RevoluteJoint>::new());
    container.insert(InspectablePropertyEditorDefinition::<PrismaticJoint>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::joint::PrismaticJoint,
    >::new());
    container.insert(InspectablePropertyEditorDefinition::<Base>::new());
    container.insert(InspectablePropertyEditorDefinition::<BaseEffect>::new());
    container.insert(InspectablePropertyEditorDefinition::<BaseLight>::new());
    container.insert(EnumPropertyEditorDefinition::<Effect>::new());
    container.insert(EnumPropertyEditorDefinition::<Emitter>::new());
    container.insert(InspectablePropertyEditorDefinition::<ReverbEffect>::new());
    container.insert(InspectablePropertyEditorDefinition::<Biquad>::new());
    container.insert(InspectablePropertyEditorDefinition::<BaseEmitter>::new());
    container.insert(InspectablePropertyEditorDefinition::<SphereEmitter>::new());
    container.insert(InspectablePropertyEditorDefinition::<CylinderEmitter>::new());
    container.insert(InspectablePropertyEditorDefinition::<CuboidEmitter>::new());
    container.insert(InspectablePropertyEditorDefinition::<PerspectiveProjection>::new());
    container.insert(InspectablePropertyEditorDefinition::<OrthographicProjection>::new());
    container.insert(InspectablePropertyEditorDefinition::<Transform>::new());
    container.insert(InspectablePropertyEditorDefinition::<CsmOptions>::new());
    container.insert(ArrayPropertyEditorDefinition::<f32, 3>::new());
    container.insert(ArrayPropertyEditorDefinition::<f32, 2>::new());
    container.insert(EnumPropertyEditorDefinition::<ColorGradingLut>::new_optional());
    container.insert(EnumPropertyEditorDefinition::<Biquad>::new_optional());
    container.insert(EnumPropertyEditorDefinition::<SkyBox>::new_optional());
    container.insert(InspectablePropertyEditorDefinition::<SkyBox>::new());
    container.insert(HandlePropertyEditorDefinition::<Node>::new(sender));
    container.insert(EnumPropertyEditorDefinition::<dim2::collider::ColliderShape>::new());
    container.insert(EnumPropertyEditorDefinition::<CoefficientCombineRule>::new());
    container.insert(EnumPropertyEditorDefinition::<CompressionOptions>::new());
    container.insert(EnumPropertyEditorDefinition::<TextureWrapMode>::new());
    container.insert(EnumPropertyEditorDefinition::<TextureMagnificationFilter>::new());
    container.insert(EnumPropertyEditorDefinition::<TextureMinificationFilter>::new());
    container.insert(EnumPropertyEditorDefinition::<Projection>::new());
    container.insert(EnumPropertyEditorDefinition::<ColliderShape>::new());
    container.insert(EnumPropertyEditorDefinition::<PropertyValue>::new());
    container.insert(EnumPropertyEditorDefinition::<Mobility>::new());
    container.insert(EnumPropertyEditorDefinition::<RigidBodyType>::new());
    container.insert(EnumPropertyEditorDefinition::<Exposure>::new());
    container.insert(EnumPropertyEditorDefinition::<RenderPath>::new());
    container.insert(EnumPropertyEditorDefinition::<FrustumSplitOptions>::new());
    container.insert(EnumPropertyEditorDefinition::<MaterialSearchOptions>::new());
    container.insert(EnumPropertyEditorDefinition::<DistanceModel>::new());
    container.insert(EnumPropertyEditorDefinition::<sound::Renderer>::new());
    container.insert(ScriptPropertyEditorDefinition {});
    container.insert(BitFieldPropertyEditorDefinition::<BitMask>::new());

    container.insert(InspectablePropertyEditorDefinition::<BallShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::collider::BallShape,
    >::new());

    container.insert(InspectablePropertyEditorDefinition::<CylinderShape>::new());

    container.insert(InspectablePropertyEditorDefinition::<ConeShape>::new());

    container.insert(InspectablePropertyEditorDefinition::<CuboidShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::collider::CuboidShape,
    >::new());

    container.insert(InspectablePropertyEditorDefinition::<CapsuleShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::collider::CapsuleShape,
    >::new());

    container.insert(InspectablePropertyEditorDefinition::<SegmentShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::collider::SegmentShape,
    >::new());

    container.insert(InspectablePropertyEditorDefinition::<TriangleShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::collider::TriangleShape,
    >::new());

    container.insert(InspectablePropertyEditorDefinition::<TrimeshShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::collider::TrimeshShape,
    >::new());

    container.insert(InspectablePropertyEditorDefinition::<HeightfieldShape>::new());
    container.insert(InspectablePropertyEditorDefinition::<
        dim2::collider::HeightfieldShape,
    >::new());

    container.insert(InspectablePropertyEditorDefinition::<ConvexPolyhedronShape>::new());

    container
}
