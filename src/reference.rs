//! Derive a reference assembly from an emitted implementation assembly.
//!
//! The current compiler emits a restricted set of C# constructs. This pass
//! preserves that emitted metadata surface, removes non-visible members when
//! possible, and replaces implementation bodies with `throw null` stubs.
use std::collections::HashSet;

use hao::{
    Module,
    dotnet::{
        entries::{CustomAttribute, Method},
        il::{Emitter, OpCode},
        md::streams::tables_stream::{FieldFlags, MethodFlags},
        module::{ModuleKind, TargetRuntime},
    },
};

fn keep_method(method: &Method, internals_visible: bool, attribute_type: bool) -> bool {
    let visibility = method.flags & MethodFlags::MemberAccessMask;
    method.flags.contains(MethodFlags::Virtual)
        || (attribute_type && method.name == ".ctor")
        || visibility == MethodFlags::Public
        || visibility == MethodFlags::Family
        || visibility == MethodFlags::FamORAssem
        || (internals_visible
            && (visibility == MethodFlags::Assem || visibility == MethodFlags::FamANDAssem))
}

fn keep_field(flags: FieldFlags, internals_visible: bool) -> bool {
    let visibility = flags & FieldFlags::FieldAccessMask;
    visibility == FieldFlags::Public
        || visibility == FieldFlags::Family
        || visibility == FieldFlags::FamORAssem
        || (internals_visible
            && (visibility == FieldFlags::Assembly || visibility == FieldFlags::FamANDAssem))
}

/// Emit a CLR reference assembly from a successfully generated implementation.
/// Both assemblies retain the same identity. The reference assembly cannot run.
pub fn emit(implementation: &[u8]) -> Result<Vec<u8>, String> {
    let mut module = Module::from_bytes(implementation)
        .map_err(|error| format!("cannot read generated assembly: {error}"))?;
    let assembly = module
        .assembly
        .clone()
        .ok_or("assembly manifest is missing")?;
    let internals_visible = assembly.get().custom_attributes.iter().any(|attribute| {
        attribute.type_name() == "System.Runtime.CompilerServices.InternalsVisibleToAttribute"
    });
    if !assembly.get().custom_attributes.iter().any(|attribute| {
        attribute.type_name() == "System.Runtime.CompilerServices.ReferenceAssemblyAttribute"
    }) {
        let constructor = module
            .corlib_attribute("System.Runtime.CompilerServices.ReferenceAssemblyAttribute")
            .map_err(|error| format!("cannot reference ReferenceAssemblyAttribute: {error}"))?;
        assembly
            .get_mut()
            .custom_attributes
            .push(CustomAttribute::without_arguments(constructor));
    }
    module.resources.clear();
    module.entry_point = None;

    for ty in module.all_types() {
        let mut ty = ty.get_mut();
        let base_name = ty.extends.as_ref().map(|base| base.full_name());
        let value_type = matches!(
            base_name.as_deref(),
            Some("System.ValueType" | "System.Enum")
        );
        let attribute_type = base_name.as_deref() == Some("System.Attribute");
        // All entries came from an existing metadata image and have stable,
        // nonzero MethodDef row IDs for the duration of this transformation.
        let retained: HashSet<u32> = ty
            .methods
            .iter()
            .filter(|method| keep_method(&method.get(), internals_visible, attribute_type))
            .map(|method| method.rid())
            .collect();
        ty.methods.retain(|method| retained.contains(&method.rid()));
        if !value_type {
            ty.fields
                .retain(|field| keep_field(field.get().flags, internals_visible));
        }
        ty.properties.retain(|property| {
            let mut property = property.get_mut();
            if property
                .getter
                .as_ref()
                .is_some_and(|method| !retained.contains(&method.rid()))
            {
                property.getter = None;
            }
            if property
                .setter
                .as_ref()
                .is_some_and(|method| !retained.contains(&method.rid()))
            {
                property.setter = None;
            }
            property
                .other_methods
                .retain(|method| retained.contains(&method.rid()));
            property.getter.is_some()
                || property.setter.is_some()
                || !property.other_methods.is_empty()
        });
        ty.events.retain(|event| {
            let mut event = event.get_mut();
            if event
                .add
                .as_ref()
                .is_some_and(|method| !retained.contains(&method.rid()))
            {
                event.add = None;
            }
            if event
                .remove
                .as_ref()
                .is_some_and(|method| !retained.contains(&method.rid()))
            {
                event.remove = None;
            }
            if event
                .fire
                .as_ref()
                .is_some_and(|method| !retained.contains(&method.rid()))
            {
                event.fire = None;
            }
            event
                .other_methods
                .retain(|method| retained.contains(&method.rid()));
            event.add.is_some()
                || event.remove.is_some()
                || event.fire.is_some()
                || !event.other_methods.is_empty()
        });
    }
    for method in module.all_methods() {
        let mut method = method.get_mut();
        if method.body.is_some() {
            method.body = Some(Emitter::build(|emitter| {
                emitter.ldnull().op(OpCode::Throw);
            }));
        }
    }
    // A loaded module retains its original PE image. Use a fresh image so
    // implementation-only IL and user strings cannot survive in old sections.
    let name = assembly.get().name.clone();
    let mut fresh = Module::new(&name, ModuleKind::Dll, TargetRuntime::NetCore(10));
    macro_rules! take {
        ($field:ident) => {
            fresh.$field = std::mem::take(&mut module.$field);
        };
    }
    take!(module);
    take!(assembly);
    take!(types);
    take!(type_refs);
    take!(type_specs);
    take!(member_refs);
    take!(method_specs);
    take!(module_refs);
    take!(assembly_refs);
    take!(files);
    take!(exported_types);
    take!(resources);
    take!(entry_point);
    take!(runtime_version);
    fresh
        .write()
        .map_err(|error| format!("cannot emit reference assembly: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hao::dotnet::{
        entries::{Field, TypeDef},
        md::streams::tables_stream::TypeAttributes,
        signature::{FieldSig, MethodSig, TypeSig},
    };
    #[test]
    fn rejects_invalid_input() {
        assert!(emit(b"not a PE").is_err());
    }

    #[test]
    fn keeps_public_property_metadata() {
        let source = "namespace Demo { public class Counter { public int Value { get; set; } } }";
        let implementation =
            crate::csharp::compile(&[("Api.cs".into(), source.into())], "Api", &[]).unwrap();
        let before = Module::from_bytes(&implementation).unwrap();
        let before_type = before.find_type("Demo", "Counter").unwrap();
        assert!(
            before_type
                .get()
                .properties
                .iter()
                .any(|property| property.get().name == "Value")
        );
        let reference = emit(&implementation).unwrap();
        let module = Module::from_bytes(&reference).unwrap();
        let ty = module.find_type("Demo", "Counter").unwrap();
        assert!(
            ty.get()
                .properties
                .iter()
                .any(|property| property.get().name == "Value")
        );
    }

    #[test]
    fn removes_private_class_members_but_retains_value_type_layout() {
        let mut module = Module::new("Api", ModuleKind::Dll, TargetRuntime::NetCore(10));
        let object = module.corlib_type("System.Object").unwrap();
        let value_type = module.corlib_type("System.ValueType").unwrap();
        let class = module.add_type(TypeDef::new(
            TypeAttributes::Public,
            "Demo",
            "Class",
            Some(object),
        ));
        let value = module.add_type(TypeDef::new(
            TypeAttributes::Public | TypeAttributes::Sealed,
            "Demo",
            "Value",
            Some(value_type),
        ));
        TypeDef::add_field(
            &class,
            Field::new(
                FieldFlags::Private,
                "Hidden",
                FieldSig { ty: TypeSig::INT32 },
            ),
        );
        TypeDef::add_field(
            &class,
            Field::new(
                FieldFlags::Public,
                "Visible",
                FieldSig { ty: TypeSig::INT32 },
            ),
        );
        TypeDef::add_field(
            &value,
            Field::new(
                FieldFlags::Private,
                "Layout",
                FieldSig { ty: TypeSig::INT32 },
            ),
        );
        for (name, flags) in [
            ("Hidden", MethodFlags::Private),
            ("Visible", MethodFlags::Public),
        ] {
            let body = Emitter::build(|emitter| {
                emitter.ret();
            });
            TypeDef::add_method(
                &class,
                Method::new(
                    flags | MethodFlags::Static,
                    name,
                    MethodSig::new_static(TypeSig::VOID, vec![]),
                )
                .with_body(body),
            );
        }
        let bytes = emit(&module.write().unwrap()).unwrap();
        let reference = Module::from_bytes(&bytes).unwrap();
        let class = reference.find_type("Demo", "Class").unwrap();
        assert!(
            class
                .get()
                .fields
                .iter()
                .all(|field| field.get().name != "Hidden")
        );
        assert!(
            class
                .get()
                .fields
                .iter()
                .any(|field| field.get().name == "Visible")
        );
        assert!(
            class
                .get()
                .methods
                .iter()
                .all(|method| method.get().name != "Hidden")
        );
        assert!(
            class
                .get()
                .methods
                .iter()
                .any(|method| method.get().name == "Visible")
        );
        let value = reference.find_type("Demo", "Value").unwrap();
        assert!(
            value
                .get()
                .fields
                .iter()
                .any(|field| field.get().name == "Layout")
        );
    }
}
