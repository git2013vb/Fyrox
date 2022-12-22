use crate::{
    core::{
        algebra::{UnitQuaternion, Vector2, Vector3, Vector4},
        math::lerpf,
        num_traits::AsPrimitive,
        reflect::{prelude::*, SetFieldByPathError},
        visitor::prelude::*,
    },
    scene::node::Node,
    utils::log::Log,
};
use std::fmt::{Debug, Display, Formatter};

#[derive(Visit, Reflect, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ValueType {
    Bool,
    F32,
    F64,
    U64,
    I64,
    U32,
    I32,
    U16,
    I16,
    U8,
    I8,

    Vector2Bool,
    Vector2F32,
    Vector2F64,
    Vector2U64,
    Vector2I64,
    Vector2U32,
    Vector2I32,
    Vector2U16,
    Vector2I16,
    Vector2U8,
    Vector2I8,

    Vector3Bool,
    Vector3F32,
    Vector3F64,
    Vector3U64,
    Vector3I64,
    Vector3U32,
    Vector3I32,
    Vector3U16,
    Vector3I16,
    Vector3U8,
    Vector3I8,

    Vector4Bool,
    Vector4F32,
    Vector4F64,
    Vector4U64,
    Vector4I64,
    Vector4U32,
    Vector4I32,
    Vector4U16,
    Vector4I16,
    Vector4U8,
    Vector4I8,

    UnitQuaternionF32,
    UnitQuaternionF64,
}

impl Default for ValueType {
    fn default() -> Self {
        Self::F32
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TrackValue {
    /// A real number.
    Real(f32),

    /// A 2-dimensional vector of real values.
    Vector2(Vector2<f32>),

    /// A 3-dimensional vector of real values.
    Vector3(Vector3<f32>),

    /// A 4-dimensional vector of real values.
    Vector4(Vector4<f32>),

    /// A quaternion that represents some rotation.
    UnitQuaternion(UnitQuaternion<f32>),
}

impl TrackValue {
    pub fn weighted_clone(&self, weight: f32) -> Self {
        match self {
            TrackValue::Real(v) => TrackValue::Real(*v * weight),
            TrackValue::Vector2(v) => TrackValue::Vector2(v.scale(weight)),
            TrackValue::Vector3(v) => TrackValue::Vector3(v.scale(weight)),
            TrackValue::Vector4(v) => TrackValue::Vector4(v.scale(weight)),
            TrackValue::UnitQuaternion(v) => TrackValue::UnitQuaternion(*v),
        }
    }

    pub fn blend_with(&mut self, other: &Self, weight: f32) {
        match (self, other) {
            (Self::Real(a), Self::Real(b)) => *a += *b * weight,
            (Self::Vector2(a), Self::Vector2(b)) => *a += b.scale(weight),
            (Self::Vector3(a), Self::Vector3(b)) => *a += b.scale(weight),
            (Self::Vector4(a), Self::Vector4(b)) => *a += b.scale(weight),
            (Self::UnitQuaternion(a), Self::UnitQuaternion(b)) => *a = a.nlerp(b, weight),
            _ => (),
        }
    }

    pub fn interpolate(&self, other: &Self, t: f32) -> Option<Self> {
        match (self, other) {
            (Self::Real(a), Self::Real(b)) => Some(Self::Real(lerpf(*a, *b, t))),
            (Self::Vector2(a), Self::Vector2(b)) => Some(Self::Vector2(a.lerp(b, t))),
            (Self::Vector3(a), Self::Vector3(b)) => Some(Self::Vector3(a.lerp(b, t))),
            (Self::Vector4(a), Self::Vector4(b)) => Some(Self::Vector4(a.lerp(b, t))),
            (Self::UnitQuaternion(a), Self::UnitQuaternion(b)) => {
                Some(Self::UnitQuaternion(a.nlerp(b, t)))
            }
            _ => None,
        }
    }

    pub fn boxed_value(&self) -> Box<dyn Reflect> {
        match self {
            TrackValue::Vector3(v) => Box::new(*v),
            TrackValue::UnitQuaternion(v) => Box::new(*v),
            TrackValue::Real(v) => Box::new(*v),
            TrackValue::Vector2(v) => Box::new(*v),
            TrackValue::Vector4(v) => Box::new(*v),
        }
    }

    pub fn numeric_type_cast(&self, value_type: ValueType) -> Option<Box<dyn Reflect>> {
        fn convert_vec2<T>(vec2: &Vector2<f32>) -> Vector2<T>
        where
            f32: AsPrimitive<T>,
            T: Copy + 'static,
        {
            Vector2::new(vec2.x.as_(), vec2.y.as_())
        }

        fn convert_vec3<T>(vec3: &Vector3<f32>) -> Vector3<T>
        where
            f32: AsPrimitive<T>,
            T: Copy + 'static,
        {
            Vector3::new(vec3.x.as_(), vec3.y.as_(), vec3.z.as_())
        }

        fn convert_vec4<T>(vec4: &Vector4<f32>) -> Vector4<T>
        where
            f32: AsPrimitive<T>,
            T: Copy + 'static,
        {
            Vector4::new(vec4.x.as_(), vec4.y.as_(), vec4.z.as_(), vec4.w.as_())
        }

        match self {
            TrackValue::Real(real) => match value_type {
                ValueType::Bool => Some(Box::new(real.ne(&0.0))),
                ValueType::F32 => Some(Box::new(*real)),
                ValueType::F64 => Some(Box::new(*real as f64)),
                ValueType::U64 => Some(Box::new(*real as u64)),
                ValueType::I64 => Some(Box::new(*real as i64)),
                ValueType::U32 => Some(Box::new(*real as u32)),
                ValueType::I32 => Some(Box::new(*real as i32)),
                ValueType::U16 => Some(Box::new(*real as u16)),
                ValueType::I16 => Some(Box::new(*real as i16)),
                ValueType::U8 => Some(Box::new(*real as u8)),
                ValueType::I8 => Some(Box::new(*real as i8)),
                _ => None,
            },
            TrackValue::Vector2(vec2) => match value_type {
                ValueType::Vector2Bool => {
                    Some(Box::new(Vector2::new(vec2.x.ne(&0.0), vec2.y.ne(&0.0))))
                }
                ValueType::Vector2F32 => Some(Box::new(*vec2)),
                ValueType::Vector2F64 => Some(Box::new(convert_vec2::<f64>(vec2))),
                ValueType::Vector2U64 => Some(Box::new(convert_vec2::<u64>(vec2))),
                ValueType::Vector2I64 => Some(Box::new(convert_vec2::<i64>(vec2))),
                ValueType::Vector2U32 => Some(Box::new(convert_vec2::<u32>(vec2))),
                ValueType::Vector2I32 => Some(Box::new(convert_vec2::<i32>(vec2))),
                ValueType::Vector2U16 => Some(Box::new(convert_vec2::<u16>(vec2))),
                ValueType::Vector2I16 => Some(Box::new(convert_vec2::<i16>(vec2))),
                ValueType::Vector2U8 => Some(Box::new(convert_vec2::<u8>(vec2))),
                ValueType::Vector2I8 => Some(Box::new(convert_vec2::<i8>(vec2))),
                _ => None,
            },
            TrackValue::Vector3(vec3) => match value_type {
                ValueType::Vector3Bool => Some(Box::new(Vector3::new(
                    vec3.x.ne(&0.0),
                    vec3.y.ne(&0.0),
                    vec3.z.ne(&0.0),
                ))),
                ValueType::Vector3F32 => Some(Box::new(*vec3)),
                ValueType::Vector3F64 => Some(Box::new(convert_vec3::<f64>(vec3))),
                ValueType::Vector3U64 => Some(Box::new(convert_vec3::<u64>(vec3))),
                ValueType::Vector3I64 => Some(Box::new(convert_vec3::<i64>(vec3))),
                ValueType::Vector3U32 => Some(Box::new(convert_vec3::<u32>(vec3))),
                ValueType::Vector3I32 => Some(Box::new(convert_vec3::<i32>(vec3))),
                ValueType::Vector3U16 => Some(Box::new(convert_vec3::<u16>(vec3))),
                ValueType::Vector3I16 => Some(Box::new(convert_vec3::<i16>(vec3))),
                ValueType::Vector3U8 => Some(Box::new(convert_vec3::<u8>(vec3))),
                ValueType::Vector3I8 => Some(Box::new(convert_vec3::<i8>(vec3))),
                _ => None,
            },
            TrackValue::Vector4(vec4) => match value_type {
                ValueType::Vector4Bool => Some(Box::new(Vector4::new(
                    vec4.x.ne(&0.0),
                    vec4.y.ne(&0.0),
                    vec4.z.ne(&0.0),
                    vec4.w.ne(&0.0),
                ))),
                ValueType::Vector4F32 => Some(Box::new(*vec4)),
                ValueType::Vector4F64 => Some(Box::new(convert_vec4::<f64>(vec4))),
                ValueType::Vector4U64 => Some(Box::new(convert_vec4::<u64>(vec4))),
                ValueType::Vector4I64 => Some(Box::new(convert_vec4::<i64>(vec4))),
                ValueType::Vector4U32 => Some(Box::new(convert_vec4::<u32>(vec4))),
                ValueType::Vector4I32 => Some(Box::new(convert_vec4::<i32>(vec4))),
                ValueType::Vector4U16 => Some(Box::new(convert_vec4::<u16>(vec4))),
                ValueType::Vector4I16 => Some(Box::new(convert_vec4::<i16>(vec4))),
                ValueType::Vector4U8 => Some(Box::new(convert_vec4::<u8>(vec4))),
                ValueType::Vector4I8 => Some(Box::new(convert_vec4::<i8>(vec4))),
                _ => None,
            },
            TrackValue::UnitQuaternion(quat) => match value_type {
                ValueType::UnitQuaternionF32 => Some(Box::new(*quat)),
                ValueType::UnitQuaternionF64 => Some(Box::new(quat.cast::<f64>())),
                _ => None,
            },
        }
    }
}

#[derive(Clone, Visit, Reflect, Debug, PartialEq, Eq)]
pub enum ValueBinding {
    Position,
    Scale,
    Rotation,
    Property { name: String, value_type: ValueType },
}

impl Display for ValueBinding {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ValueBinding::Position => write!(f, "Position"),
            ValueBinding::Scale => write!(f, "Scale"),
            ValueBinding::Rotation => write!(f, "Rotation"),
            ValueBinding::Property { name, .. } => write!(f, "{}", name),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BoundValue {
    pub binding: ValueBinding,
    pub value: TrackValue,
}

impl BoundValue {
    pub fn weighted_clone(&self, weight: f32) -> Self {
        Self {
            binding: self.binding.clone(),
            value: self.value.weighted_clone(weight),
        }
    }

    pub fn blend_with(&mut self, other: &Self, weight: f32) {
        assert_eq!(self.binding, other.binding);
        self.value.blend_with(&other.value, weight);
    }

    pub fn interpolate(&self, other: &Self, t: f32) -> Option<Self> {
        assert_eq!(self.binding, other.binding);
        self.value.interpolate(&other.value, t).map(|value| Self {
            binding: self.binding.clone(),
            value,
        })
    }

    pub fn boxed_value(&self) -> Box<dyn Reflect> {
        self.value.boxed_value()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoundValueCollection {
    pub values: Vec<BoundValue>,
}

impl BoundValueCollection {
    pub fn weighted_clone(&self, weight: f32) -> Self {
        Self {
            values: self
                .values
                .iter()
                .map(|v| v.weighted_clone(weight))
                .collect::<Vec<_>>(),
        }
    }

    pub fn blend_with(&mut self, other: &Self, weight: f32) {
        for value in self.values.iter_mut() {
            if let Some(other_value) = other.values.iter().find(|v| v.binding == value.binding) {
                value.blend_with(other_value, weight);
            }
        }
    }

    pub fn interpolate(&self, other: &Self, t: f32) -> Self {
        let mut new_values = Vec::new();
        for value in self.values.iter() {
            if let Some(other_value) = other.values.iter().find(|v| v.binding == value.binding) {
                new_values.push(value.interpolate(other_value, t).unwrap());
            }
        }

        Self { values: new_values }
    }

    pub fn apply(&self, node_ref: &mut Node) {
        for bound_value in self.values.iter() {
            match bound_value.binding {
                ValueBinding::Position => {
                    if let TrackValue::Vector3(v) = bound_value.value {
                        node_ref.local_transform_mut().set_position(v);
                    } else {
                        Log::err(
                            "Unable to apply position, because underlying type is not Vector3!",
                        )
                    }
                }
                ValueBinding::Scale => {
                    if let TrackValue::Vector3(v) = bound_value.value {
                        node_ref.local_transform_mut().set_scale(v);
                    } else {
                        Log::err("Unable to apply scaling, because underlying type is not Vector3!")
                    }
                }
                ValueBinding::Rotation => {
                    if let TrackValue::UnitQuaternion(v) = bound_value.value {
                        node_ref.local_transform_mut().set_rotation(v);
                    } else {
                        Log::err("Unable to apply rotation, because underlying type is not UnitQuaternion!")
                    }
                }
                ValueBinding::Property {
                    name: ref property_name,
                    value_type,
                } => {
                    if let Some(casted) = bound_value.value.numeric_type_cast(value_type) {
                        if let Err(err) = node_ref
                            .as_reflect_mut()
                            .set_field_by_path(property_name, casted)
                        {
                            match err {
                                SetFieldByPathError::InvalidPath { reason, .. } => {
                                    Log::err(format!(
                                        "Failed to set property {}! Invalid path: {}",
                                        property_name, reason
                                    ));
                                }
                                SetFieldByPathError::InvalidValue(_) => {
                                    Log::err(format!(
                                        "Failed to set property {}! Types mismatch!",
                                        property_name
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
