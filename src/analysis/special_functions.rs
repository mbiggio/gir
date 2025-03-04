use crate::{
    analysis::{
        functions::{Info as FuncInfo, Visibility},
        imports::Imports,
    },
    config::GObject,
    library::{Type as LibType, TypeId},
    version::Version,
};
use std::{collections::BTreeMap, str::FromStr};

#[derive(Clone, Copy, Eq, Debug, Ord, PartialEq, PartialOrd)]
pub enum Type {
    Compare,
    Copy,
    Equal,
    Free,
    Ref,
    Display,
    Unref,
    Hash,
}

impl FromStr for Type {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        use self::Type::*;
        match s {
            "compare" => Ok(Compare),
            "copy" => Ok(Copy),
            "equal" => Ok(Equal),
            "free" | "destroy" => Ok(Free),
            "is_equal" => Ok(Equal),
            "ref" | "ref_" => Ok(Ref),
            "unref" => Ok(Unref),
            "hash" => Ok(Hash),
            _ => Err(format!("Unknown type '{}'", s)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TraitInfo {
    pub glib_name: String,
    pub version: Option<Version>,
}

type TraitInfos = BTreeMap<Type, TraitInfo>;

#[derive(Clone, Copy, Eq, Debug, Ord, PartialEq, PartialOrd)]
pub enum FunctionType {
    StaticStringify,
}

#[derive(Debug, Clone)]
pub struct FunctionInfo {
    pub type_: FunctionType,
    pub version: Option<Version>,
}

type FunctionInfos = BTreeMap<String, FunctionInfo>;

#[derive(Debug, Default)]
pub struct Infos {
    traits: TraitInfos,
    functions: FunctionInfos,
}

impl Infos {
    pub fn traits(&self) -> &TraitInfos {
        &self.traits
    }

    pub fn traits_mut(&mut self) -> &mut TraitInfos {
        &mut self.traits
    }

    pub fn has_trait(&self, type_: Type) -> bool {
        self.traits.contains_key(&type_)
    }

    pub fn functions(&self) -> &FunctionInfos {
        &self.functions
    }
}

/// Returns true on functions that take an instance as single argument and
/// return a string as result.
fn is_stringify(func: &mut FuncInfo, parent_type: &LibType, obj: &GObject) -> bool {
    if func.parameters.c_parameters.len() != 1 {
        return false;
    }
    if !func.parameters.c_parameters[0].instance_parameter {
        return false;
    }

    if let Some(ret) = func.ret.parameter.as_mut() {
        if ret.typ != TypeId::tid_utf8() {
            return false;
        }

        if func.name == "to_string" {
            // Rename to to_str to make sure it doesn't clash with ToString::to_string
            func.name = "to_str".to_owned();

            // As to not change old code behaviour, assume non-nullability outside
            // enums and flags only, and exclusively for to_string. Function inside
            // enums and flags have been appropriately marked in Gir.
            if !obj.trust_return_value_nullability
                && !matches!(parent_type, LibType::Enumeration(_) | LibType::Bitfield(_))
            {
                *ret.nullable = false;
            }
        }

        // Cannot generate Display implementation for Option<>
        !*ret.nullable
    } else {
        false
    }
}

fn update_func(func: &mut FuncInfo, type_: Type) -> bool {
    if func.visibility != Visibility::Comment {
        func.visibility = visibility(type_);
    }
    true
}

pub fn extract(functions: &mut Vec<FuncInfo>, parent_type: &LibType, obj: &GObject) -> Infos {
    let mut specials = Infos::default();
    let mut has_copy = false;
    let mut has_free = false;
    let mut destroy = None;

    for (pos, func) in functions.iter_mut().enumerate() {
        if is_stringify(func, parent_type, obj) {
            let return_transfer_none = func
                .ret
                .parameter
                .as_ref()
                .map_or(false, |ret| ret.transfer == crate::library::Transfer::None);

            // Assume only enumerations and bitfields can return static strings
            let returns_static_ref = return_transfer_none
                && matches!(parent_type, LibType::Enumeration(_) | LibType::Bitfield(_))
                // We cannot mandate returned lifetime if this is not generated.
                // (And this prevents an unused std::ffi::CStr from being emitted below)
                && func.status.need_generate();

            if returns_static_ref {
                // Override the function with a &'static (non allocating) -returning string
                // if the transfer type is none and it matches the above heuristics.
                specials.functions.insert(
                    func.glib_name.clone(),
                    FunctionInfo {
                        type_: FunctionType::StaticStringify,
                        version: func.version,
                    },
                );
            }

            // Some stringifying functions can serve as Display implementation
            if matches!(
                func.name.as_str(),
                "to_string" | "to_str" | "name" | "get_name"
            ) {
                // FUTURE: Decide which function gets precedence if multiple Display prospects exist.
                specials.traits.insert(
                    Type::Display,
                    TraitInfo {
                        glib_name: func.glib_name.clone(),
                        version: func.version,
                    },
                );
            }
        } else if let Ok(type_) = func.name.parse() {
            if func.name == "destroy" {
                destroy = Some((func.glib_name.clone(), pos));
                continue;
            }
            if !update_func(func, type_) {
                continue;
            }
            if func.name == "copy" {
                has_copy = true;
            } else if func.name == "free" {
                has_free = true;
            }

            specials.traits.insert(
                type_,
                TraitInfo {
                    glib_name: func.glib_name.clone(),
                    version: func.version,
                },
            );
        }
    }

    if has_copy && !has_free {
        if let Some((glib_name, pos)) = destroy {
            let ty_ = Type::from_str("destroy").unwrap();
            let func = &mut functions[pos];
            update_func(func, ty_);
            specials.traits.insert(
                ty_,
                TraitInfo {
                    glib_name,
                    version: func.version,
                },
            );
        }
    }

    specials
}

fn visibility(t: Type) -> Visibility {
    use self::Type::*;
    match t {
        Copy | Free | Ref | Unref => Visibility::Hidden,
        Hash | Compare | Equal => Visibility::Private,
        Display => Visibility::Public,
    }
}

// Some special functions (e.g. `copy` on refcounted types) should be exposed
pub fn unhide(functions: &mut Vec<FuncInfo>, specials: &Infos, type_: Type) {
    if let Some(func) = specials.traits().get(&type_) {
        let func = functions
            .iter_mut()
            .find(|f| f.glib_name == func.glib_name && f.visibility != Visibility::Comment);
        if let Some(func) = func {
            func.visibility = Visibility::Public;
        }
    }
}

pub fn analyze_imports(specials: &Infos, imports: &mut Imports) {
    for (type_, info) in specials.traits() {
        use self::Type::*;
        match *type_ {
            Compare => imports.add_with_version("std::cmp", info.version),
            Display => imports.add_with_version("std::fmt", info.version),
            Hash => imports.add_with_version("std::hash", info.version),
            _ => {}
        }
    }
    for info in specials.functions().values() {
        match info.type_ {
            FunctionType::StaticStringify => {
                imports.add_with_version("std::ffi::CStr", info.version)
            }
        }
    }
}
