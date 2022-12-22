//! Parameter is a name variable of a fixed type. See [`Parameter`] docs for more info.

use crate::core::{reflect::prelude::*, visitor::prelude::*};
use fxhash::FxHashMap;
use std::{
    cell::{Cell, RefCell},
    ops::{Deref, DerefMut},
};
use strum_macros::{AsRefStr, EnumString, EnumVariantNames};

/// Machine parameter is a named variable of a fixed type. Machine uses various parameters for specific actions. For example
/// Rule parameter is used to check where transition from a state to state is possible, `Weight` parameters are used to be
/// a source real numbers that are used to calculate blend weights, etc.
#[derive(Copy, Clone, Debug, PartialEq, Reflect, Visit, EnumVariantNames, EnumString, AsRefStr)]
pub enum Parameter {
    /// Weight parameter is used to control blend weight in animation blending nodes.
    Weight(f32),

    /// Rule parameter is used to check where transition from a state to state is possible.
    Rule(bool),

    /// An index of a pose.
    Index(u32),
}

impl Default for Parameter {
    fn default() -> Self {
        Self::Weight(0.0)
    }
}

/// Specific animation pose weight.
#[derive(Debug, Visit, Clone, PartialEq, Reflect, EnumVariantNames, EnumString, AsRefStr)]
pub enum PoseWeight {
    /// Fixed scalar value. Should not be negative, negative numbers will probably result in weird visual artifacts.
    Constant(f32),

    /// Reference to Weight parameter with given name.
    Parameter(String),
}

impl Default for PoseWeight {
    fn default() -> Self {
        Self::Constant(0.0)
    }
}

/// A parameter value with its name.
#[derive(Reflect, Visit, Default, Debug, Clone, PartialEq)]
pub struct ParameterDefinition {
    /// Name of the parameter.
    pub name: String,

    /// Value of the parameter.
    pub value: Parameter,
}

#[derive(Default, Debug, Clone)]
struct Wrapper {
    parameters: Vec<ParameterDefinition>,
    dirty: Cell<bool>,
}

impl PartialEq for Wrapper {
    fn eq(&self, other: &Self) -> bool {
        self.parameters == other.parameters
    }
}

impl Visit for Wrapper {
    fn visit(&mut self, name: &str, visitor: &mut Visitor) -> VisitResult {
        self.parameters.visit(name, visitor)
    }
}

impl Deref for Wrapper {
    type Target = Vec<ParameterDefinition>;

    fn deref(&self) -> &Self::Target {
        &self.parameters
    }
}

impl DerefMut for Wrapper {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dirty.set(true);
        &mut self.parameters
    }
}

/// A container for all parameters used by a state machine. Parameters are shared across multiple animation layers.
#[derive(Reflect, Visit, Default, Debug)]
pub struct ParameterContainer {
    #[reflect(deref)]
    parameters: Wrapper,

    #[reflect(hidden)]
    #[visit(skip)]
    lookup: RefCell<FxHashMap<String, usize>>,
}

impl PartialEq for ParameterContainer {
    fn eq(&self, other: &Self) -> bool {
        self.parameters == other.parameters
    }
}

impl Clone for ParameterContainer {
    fn clone(&self) -> Self {
        Self {
            parameters: self.parameters.clone(),
            lookup: RefCell::new(self.lookup.borrow().clone()),
        }
    }
}

impl ParameterContainer {
    fn update_index(&self) {
        if self.parameters.dirty.get() {
            *self.lookup.borrow_mut() = self
                .parameters
                .parameters
                .iter()
                .enumerate()
                .map(|(i, p)| (p.name.clone(), i))
                .collect();
            self.parameters.dirty.set(false);
        }
    }

    /// Adds a new parameter with a given name and value to the container.
    pub fn add(&mut self, name: &str, value: Parameter) {
        self.parameters.push(ParameterDefinition {
            name: name.to_string(),
            value,
        })
    }

    /// Tries to borrow a parameter by its name. The method has O(1) complexity.
    pub fn get(&self, name: &str) -> Option<&Parameter> {
        self.update_index();
        self.lookup
            .borrow()
            .get(name)
            .and_then(|i| self.parameters.parameters.get(*i).map(|d| &d.value))
    }

    /// Tries to borrow a parameter by its name. The method has O(1) complexity.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Parameter> {
        self.update_index();
        self.lookup
            .borrow()
            .get(name)
            .and_then(|i| self.parameters.parameters.get_mut(*i).map(|d| &mut d.value))
    }
}
