use std::io::{Result, Write};

use analysis;
use analysis::namespaces::MAIN;
use case::CaseExt;
use env::Env;
use file_saver::save_to_file;
use library::*;
use library::Type as LType;
use nameutil;
use regex::{Captures, Regex};
use self::format::reformat_doc;
use stripper_lib::Type as SType;
use stripper_lib::{TypeStruct, write_file_name, write_item_doc};
use traits::*;

mod format;

trait ToStripperType {
    fn to_stripper_type(&self) -> TypeStruct;
}

macro_rules! impl_to_stripper_type {
    ($ty:ident, $enum_var:ident) => {
        impl ToStripperType for $ty {
            fn to_stripper_type(&self) -> TypeStruct {
                TypeStruct::new(SType::$enum_var, &self.name)
            }
        }
    }
}

impl_to_stripper_type!(Member, Variant);
impl_to_stripper_type!(Enumeration, Enum);
impl_to_stripper_type!(Bitfield, Flags);
impl_to_stripper_type!(Record, Struct);
impl_to_stripper_type!(Function, Fn);
impl_to_stripper_type!(Class, Struct);

pub fn generate(env: &Env) {
    let path =  env.config.target_path.join("docs.md");
    println!("Generating documentation {:?}", path);
    save_to_file(&path, env.config.make_backup,
        |w| generate_doc(w, &env));
}

fn generate_doc(mut w: &mut Write, env: &Env) -> Result<()> {
    try!(write_file_name(w, None));

    for (tid, type_) in env.library.namespace_types(MAIN) {
        let obj = env.config.objects.get(&tid.full_name(&env.library))
            .and_then(|obj| if obj.status.ignored() { None } else { Some(obj) });
        match *type_ {
            LType::Class(..) => {
                if let Some(obj) = obj {
                    try!(create_object_doc(w, env, &analysis::object::class(env, obj).unwrap()));
                }
            }
            LType::Interface(..) => {
                if let Some(obj) = obj {
                    try!(create_object_doc(w, env,
                                           &analysis::object::interface(env, obj).unwrap()));
                }
            }
            LType::Record(..) => {
                if let Some(obj) = obj {
                    try!(create_record_doc(w, env, &analysis::record::new(env, obj).unwrap()));
                }
            }
            _ => {
                try!(handle_type(&mut w, env, type_));
            }
        }
    }

    Ok(())
}

fn handle_type(w: &mut Write, env: &Env, ty: &LType) -> Result<()> {
    match *ty {
        LType::Bitfield(ref e) => create_bitfield_doc(w, env, &e),
        LType::Enumeration(ref e) => create_enum_doc(w, env, &e),
        _ => Ok(()),
    }
}

fn create_object_doc(w: &mut Write, env: &Env, info: &analysis::object::Info) -> Result<()> {
    let symbols = env.symbols.borrow();
    let ty = TypeStruct::new(SType::Struct, &info.name);
    let ty_ext = TypeStruct::new(SType::Trait, &format!("{}Ext", info.name));
    let has_trait = info.has_children;
    let doc;
    let functions: &[Function];

    match *env.library.type_(info.type_id) {
        Type::Class(ref cl) => {
            doc = cl.doc.as_ref();
            functions = &cl.functions;
        }
        Type::Interface(ref iface) => {
            doc = iface.doc.as_ref();
            functions = &iface.functions;
        }
        _ => unreachable!(),
    }

    try!(write_item_doc(w, &ty, |w| {
        if let Some(ver) = info.deprecated_version {
            try!(write!(w, "`[Deprecated since {}]` ", ver));
        }
        if let Some(doc) = doc {
            try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
        } else {
            try!(writeln!(w, ""));
        }
        if let Some(version) = info.version {
            try!(writeln!(w, "\nSince: {}", version));
        }

        let impl_self = if has_trait { Some(info.type_id) } else { None };
        let implements = impl_self.iter()
            .chain(env.class_hierarchy.supertypes(info.type_id))
            .filter(|&tid| !env.type_status(&tid.full_name(&env.library)).ignored())
            .map(|&tid| format!("[`{name}Ext`](trait.{name}Ext.html)",
                                name = env.library.type_(tid).get_name()))
            .collect::<Vec<_>>();
        if !implements.is_empty() {
            try!(writeln!(w, "\n# Implements\n"));
            try!(writeln!(w, "{}", &implements.join(", ")));
        }
        Ok(())
    }));

    if has_trait {
        try!(write_item_doc(w, &ty_ext, |w| {
            if let Some(ver) = info.deprecated_version {
                try!(write!(w, "`[Deprecated since {}]` ", ver));
            }
            try!(writeln!(w, "Trait containing all `{}` methods.", ty.name));

            if let Some(version) = info.version {
                try!(writeln!(w, "\nSince: {}", version));
            }

            let mut implementors = Some(info.type_id).into_iter()
                .chain(env.class_hierarchy.subtypes(info.type_id))
                .filter(|&tid| !env.type_status(&tid.full_name(&env.library)).ignored())
                .map(|tid| format!("[`{name}`](struct.{name}.html)",
                                    name = env.library.type_(tid).get_name()))
                .collect::<Vec<_>>();
            implementors.sort();

            try!(writeln!(w, "\n# Implementors\n"));
            try!(writeln!(w, "{}", implementors.join(", ")));
            Ok(())
        }));
    }

    let ty = TypeStruct { ty: SType::Impl, ..ty };

    for function in functions {
        let ty = if has_trait && function.parameters.iter().any(|p| p.instance_parameter) {
            ty_ext.clone()
        }
        else {
            ty.clone()
        };
        try!(create_fn_doc(w, env, &function, Some(Box::new(ty))));
    }
    Ok(())
}

fn create_record_doc(w: &mut Write, env: &Env, info: &analysis::record::Info) -> Result<()> {
    let record: &Record = env.library.type_(info.type_id).to_ref_as();
    let ty = record.to_stripper_type();
    let symbols = env.symbols.borrow();

    try!(write_item_doc(w, &ty, |w| {
        if let Some(ref doc) = record.doc {
            if let Some(ver) = info.deprecated_version {
                try!(write!(w, "`[Deprecated since {}]` ", ver));
            }
            try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
        }
        if let Some(ver) = info.deprecated_version {
            try!(writeln!(w, "\n# Deprecated since {}\n", ver));
        } else if record.doc_deprecated.is_some() {
            try!(writeln!(w, "\n# Deprecated\n"));
        }
        if let Some(ref doc) = record.doc_deprecated {
            try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
        }
        if let Some(version) = info.version {
            try!(writeln!(w, "\nSince: {}", version));
        }
        Ok(())
    }));

    let ty = TypeStruct { ty: SType::Impl, ..ty };
    for function in &record.functions {
        try!(create_fn_doc(w, env, &function, Some(Box::new(ty.clone()))));
    }
    Ok(())
}

fn create_bitfield_doc(w: &mut Write, env: &Env, bitfield: &Bitfield) -> Result<()> {
    let ty = TypeStruct { name: bitfield.c_type.clone(), ..bitfield.to_stripper_type() };
    let symbols = env.symbols.borrow();

    try!(write_item_doc(w, &ty, |w| {
        if let Some(ref doc) = bitfield.doc {
            try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
        }
        if let Some(ver) = bitfield.deprecated_version {
            try!(writeln!(w, "\n# Deprecated since {}\n", ver));
        } else if bitfield.doc_deprecated.is_some() {
            try!(writeln!(w, "\n# Deprecated\n"));
        }
        if let Some(ref doc) = bitfield.doc_deprecated {
            try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
        }
        Ok(())
    }));

    for member in bitfield.members.iter() {
        let sub_ty = TypeStruct {
            name: member.c_identifier.clone(),
            ty: SType::Const,
            parent: Some(Box::new(ty.clone())),
            args: Vec::new(),
        };

        if member.doc.is_some() {
            try!(write_item_doc(w, &sub_ty, |w| {
                if let Some(ref doc) = member.doc {
                    try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
                }
                Ok(())
            }));
        }
    }

    if let Some(version) = bitfield.version {
        if version > env.config.min_cfg_version {
            try!(writeln!(w, "\nSince: {}\n", version));
        }
    }
    Ok(())
}

fn create_enum_doc(w: &mut Write, env: &Env, enum_: &Enumeration) -> Result<()> {
    let ty = TypeStruct { name: enum_.c_type.clone(), ..enum_.to_stripper_type() };
    let symbols = env.symbols.borrow();

    try!(write_item_doc(w, &ty, |w| {
        if let Some(ref doc) = enum_.doc {
            try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
        }
        if let Some(ver) = enum_.deprecated_version {
            try!(writeln!(w, "\n# Deprecated since {}\n", ver));
        } else if enum_.doc_deprecated.is_some() {
            try!(writeln!(w, "\n# Deprecated\n"));
        }
        if let Some(ref doc) = enum_.doc_deprecated {
            try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
        }
        Ok(())
    }));

    for member in enum_.members.iter() {
        let mut sub_ty = TypeStruct { name: member.name.to_camel(), ..member.to_stripper_type()};

        if member.doc.is_some() {
            sub_ty.parent = Some(Box::new(ty.clone()));
            try!(write_item_doc(w, &sub_ty, |w| {
                if let Some(ref doc) = member.doc {
                    try!(writeln!(w, "{}", reformat_doc(doc, &symbols)));
                }
                Ok(())
            }));
        }
    }

    if let Some(version) = enum_.version {
        if version > env.config.min_cfg_version {
            try!(writeln!(w, "\nSince: {}\n", version));
        }
    }
    Ok(())
}

lazy_static! {
    static ref PARAM_NAME: Regex = Regex::new(r"@(\w+)\b").unwrap();
}

fn fix_param_names(doc: &str, self_name: &Option<String>) -> String {
    PARAM_NAME.replace_all(doc, |caps: &Captures| {
        if let Some(ref self_name) = *self_name {
            if &caps[1] == self_name {
                return "@self".into()
            }
        }
        format!("@{}", nameutil::mangle_keywords(&caps[1]))
    })
}

fn create_fn_doc(w: &mut Write, env: &Env, fn_: &Function, parent: Option<Box<TypeStruct>>)
        -> Result<()> {
    if fn_.doc.is_none() && fn_.doc_deprecated.is_none() && fn_.ret.doc.is_none() {
        if fn_.parameters.iter().all(|p| {
            p.doc.is_none()
        }) {
            return Ok(());
        }
    }

    let symbols = env.symbols.borrow();
    let ty = TypeStruct { parent: parent, ..fn_.to_stripper_type() };

    let self_name: Option<String> = fn_.parameters.iter()
        .find(|p| p.instance_parameter)
        .map(|p| p.name.clone());

    write_item_doc(w, &ty, |w| {
        if let Some(ref doc) = fn_.doc {
            try!(writeln!(w, "{}", reformat_doc(&fix_param_names(doc, &self_name), &symbols)));
        }
        if let Some(version) = fn_.version {
            if version > env.config.min_cfg_version {
                try!(writeln!(w, "\nSince: {}\n", version));
            }
        }
        if let Some(ver) = fn_.deprecated_version {
            try!(writeln!(w, "\n# Deprecated since {}\n", ver));
        } else if fn_.doc_deprecated.is_some() {
            try!(writeln!(w, "\n# Deprecated\n"));
        }
        if let Some(ref doc) = fn_.doc_deprecated {
            try!(writeln!(w, "{}", reformat_doc(&fix_param_names(doc, &self_name), &symbols)));
        }

        for parameter in fn_.parameters.iter() {
            if parameter.instance_parameter || parameter.name.is_empty() {
                continue
            }
            if let Some(ref doc) = parameter.doc {
                try!(writeln!(w, "## `{}`", nameutil::mangle_keywords(&parameter.name[..])));
                try!(writeln!(w, "{}", reformat_doc(&fix_param_names(doc, &self_name), &symbols)));
            }
        }

        if let Some(ref doc) = fn_.ret.doc {
            try!(writeln!(w, "\n# Returns\n"));
            try!(writeln!(w, "{}", reformat_doc(&fix_param_names(doc, &self_name), &symbols)));
        }
        Ok(())
    })
}
