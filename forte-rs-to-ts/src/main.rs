#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_span;

use rustc_driver::{Callbacks, Compilation, run_compiler};
use rustc_hir::def::{CtorKind, DefKind};
use rustc_interface::interface::Compiler;
use rustc_middle::ty::{AdtDef, GenericArgsRef, Ty, TyCtxt, Visibility};
use rustc_span::def_id::DefId;
use std::collections::{HashMap, HashSet};
use std::env;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

#[derive(Debug, Clone)]
enum TsType {
    Primitive(String),
    Array(Box<TsType>),
    Tuple(Vec<TsType>),
    Union(Vec<TsType>),
    Object(Vec<TsField>),
    Undefined(Box<TsType>),
    Reference(String),
}

#[derive(Debug, Clone)]
struct TsField {
    name: String,
    ty: TsType,
    is_optional: bool,
}

#[derive(Debug, Clone)]
struct TsDefinition {
    /// Original: "route_generated::pages_index::UserProfile"
    full_path: String,
    /// Resolved: [] or ["utils"]
    namespace: Vec<String>,
    /// Resolved: "UserProfile" or "Review"
    type_name: String,
    ty: TsType,
}

fn strip_undefined(ty: &TsType) -> String {
    match ty {
        TsType::Undefined(inner) => format!("{}", inner),
        _ => format!("{}", ty),
    }
}

impl fmt::Display for TsType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TsType::Primitive(s) => write!(f, "{}", s),
            TsType::Array(inner) => write!(f, "{}[]", inner),
            TsType::Tuple(types) => {
                write!(f, "[")?;
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{}", ty)?;
                }
                write!(f, "]")
            }
            TsType::Union(types) => {
                for (i, ty) in types.iter().enumerate() {
                    if i > 0 {
                        write!(f, " | ")?;
                    }
                    write!(f, "{}", ty)?;
                }
                Ok(())
            }
            TsType::Object(fields) => {
                write!(f, "{{ ")?;
                for (i, field) in fields.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    let optional_marker = if field.is_optional { "?" } else { "" };
                    let ty_str = if field.is_optional {
                        strip_undefined(&field.ty)
                    } else {
                        format!("{}", field.ty)
                    };
                    write!(f, "{}{}: {}", field.name, optional_marker, ty_str)?;
                }
                write!(f, " }}")
            }
            TsType::Undefined(inner) => write!(f, "{} | undefined", inner),
            TsType::Reference(name) => write!(f, "{}", name),
        }
    }
}

fn snake_to_camel(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = false;

    for ch in s.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }

    result
}

struct TypeConverter<'tcx> {
    tcx: TyCtxt<'tcx>,
    visited: HashSet<DefId>,
    definitions: Vec<TsDefinition>,
}

impl<'tcx> TypeConverter<'tcx> {
    fn new(tcx: TyCtxt<'tcx>) -> Self {
        Self {
            tcx,
            visited: HashSet::new(),
            definitions: Vec::new(),
        }
    }

    fn convert_type(&mut self, ty: Ty<'tcx>, context: &str) -> TsType {
        match ty.kind() {
            rustc_middle::ty::TyKind::Bool => TsType::Primitive("boolean".to_string()),
            rustc_middle::ty::TyKind::Int(_)
            | rustc_middle::ty::TyKind::Uint(_)
            | rustc_middle::ty::TyKind::Float(_) => TsType::Primitive("number".to_string()),
            rustc_middle::ty::TyKind::Str => TsType::Primitive("string".to_string()),
            rustc_middle::ty::TyKind::Char => TsType::Primitive("string".to_string()),
            rustc_middle::ty::TyKind::Ref(_, inner_ty, _) => self.convert_type(*inner_ty, context),
            rustc_middle::ty::TyKind::Array(inner_ty, _)
            | rustc_middle::ty::TyKind::Slice(inner_ty) => {
                TsType::Array(Box::new(self.convert_type(*inner_ty, context)))
            }
            rustc_middle::ty::TyKind::Tuple(types) => {
                let converted: Vec<TsType> = types
                    .iter()
                    .map(|t| self.convert_type(t, context))
                    .collect();
                TsType::Tuple(converted)
            }
            rustc_middle::ty::TyKind::Adt(adt_def, substs) => {
                if self.is_std_type(adt_def, "String") {
                    TsType::Primitive("string".to_string())
                } else if self.is_std_type(adt_def, "Option") {
                    let inner_ty = substs[0].expect_ty();
                    TsType::Undefined(Box::new(self.convert_type(inner_ty, context)))
                } else if self.is_std_type(adt_def, "Vec") {
                    let inner_ty = substs[0].expect_ty();
                    TsType::Array(Box::new(self.convert_type(inner_ty, context)))
                } else if self.is_std_type(adt_def, "HashMap")
                    || self.is_std_type(adt_def, "BTreeMap")
                {
                    let val_ty = substs[1].expect_ty();
                    TsType::Object(vec![TsField {
                        name: "[key: string]".to_string(),
                        ty: self.convert_type(val_ty, context),
                        is_optional: false,
                    }])
                } else if self.is_std_type(adt_def, "HashSet")
                    || self.is_std_type(adt_def, "BTreeSet")
                {
                    let inner_ty = substs[0].expect_ty();
                    TsType::Array(Box::new(self.convert_type(inner_ty, context)))
                } else if self.is_std_type(adt_def, "Box")
                    || self.is_std_type(adt_def, "Rc")
                    || self.is_std_type(adt_def, "Arc")
                {
                    let inner_ty = substs[0].expect_ty();
                    self.convert_type(inner_ty, context)
                } else {
                    self.convert_adt(adt_def, substs, context)
                }
            }
            _ => {
                eprintln!("Error: Unsupported type: {:?} in {}", ty.kind(), context);
                std::process::exit(1);
            }
        }
    }

    fn is_std_type(&self, adt_def: &AdtDef<'tcx>, name: &str) -> bool {
        let def_path = self.tcx.def_path_str(adt_def.did());
        def_path == format!("std::string::{}", name)
            || def_path == format!("alloc::string::{}", name)
            || def_path == format!("std::option::{}", name)
            || def_path == format!("core::option::{}", name)
            || def_path == format!("std::vec::{}", name)
            || def_path == format!("alloc::vec::{}", name)
            || def_path == format!("std::collections::hash::map::{}", name)
            || def_path == format!("std::collections::hash::set::{}", name)
            || def_path == format!("std::collections::btree::map::{}", name)
            || def_path == format!("std::collections::btree::set::{}", name)
            || def_path == format!("alloc::collections::btree::map::{}", name)
            || def_path == format!("alloc::collections::btree::set::{}", name)
            || def_path == format!("std::boxed::{}", name)
            || def_path == format!("alloc::boxed::{}", name)
            || def_path == format!("std::rc::{}", name)
            || def_path == format!("alloc::rc::{}", name)
            || def_path == format!("std::sync::{}", name)
            || def_path == format!("alloc::sync::{}", name)
    }

    fn convert_adt(
        &mut self,
        adt_def: &AdtDef<'tcx>,
        substs: GenericArgsRef<'tcx>,
        context: &str,
    ) -> TsType {
        let def_id = adt_def.did();
        let type_name = self.tcx.def_path_str(def_id);

        if !self.visited.insert(def_id) {
            return TsType::Reference(type_name.clone());
        }

        let ts_type = if adt_def.is_struct() {
            self.convert_struct(def_id, substs, context)
        } else if adt_def.is_enum() {
            self.convert_enum(def_id, substs, adt_def, context)
        } else {
            eprintln!("Error: Unsupported ADT type: {} in {}", type_name, context);
            std::process::exit(1);
        };

        self.visited.remove(&def_id);
        ts_type
    }

    fn convert_struct(
        &mut self,
        def_id: DefId,
        substs: GenericArgsRef<'tcx>,
        context: &str,
    ) -> TsType {
        let type_name = self.tcx.def_path_str(def_id);
        let fields: Vec<TsField> = self
            .tcx
            .adt_def(def_id)
            .all_fields()
            .map(|field| {
                let field_name = self.tcx.item_name(field.did).to_string();
                let field_name_camel = snake_to_camel(&field_name);
                let field_ty = field.ty(self.tcx, substs);
                let field_context = format!("{}.{}", context, field_name);

                let (is_optional, actual_ty) =
                    if let rustc_middle::ty::TyKind::Adt(adt_def, substs) = field_ty.kind() {
                        if self.is_std_type(adt_def, "Option") {
                            let inner_ty = substs[0].expect_ty();
                            (true, self.convert_type(inner_ty, &field_context))
                        } else {
                            (false, self.convert_type(field_ty, &field_context))
                        }
                    } else {
                        (false, self.convert_type(field_ty, &field_context))
                    };

                TsField {
                    name: field_name_camel,
                    ty: actual_ty,
                    is_optional,
                }
            })
            .collect();

        let ts_type = TsType::Object(fields.clone());
        self.definitions.push(TsDefinition {
            full_path: type_name.clone(),
            namespace: vec![],            // Will be resolved later
            type_name: type_name.clone(), // Will be resolved later
            ty: ts_type.clone(),
        });
        ts_type
    }

    fn convert_enum(
        &mut self,
        _def_id: DefId,
        substs: GenericArgsRef<'tcx>,
        adt_def: &AdtDef<'tcx>,
        context: &str,
    ) -> TsType {
        let mut variants: Vec<TsType> = Vec::new();

        for variant in adt_def.variants() {
            let variant_ty = if variant.fields.is_empty() {
                TsType::Object(vec![TsField {
                    name: "t".to_string(),
                    ty: TsType::Primitive(format!("\"{}\"", variant.name)),
                    is_optional: false,
                }])
            } else {
                let is_tuple_variant = variant.ctor_kind() == Some(CtorKind::Fn);
                let fields: Vec<TsField> = variant
                    .fields
                    .iter()
                    .enumerate()
                    .map(|(i, field)| {
                        let field_name = if is_tuple_variant {
                            format!("_{}", i)
                        } else {
                            let rust_name = self.tcx.item_name(field.did).to_string();
                            snake_to_camel(&rust_name)
                        };
                        let field_ty = field.ty(self.tcx, substs);
                        let field_context = format!("{}::{}.{}", context, variant.name, field_name);

                        let (is_optional, actual_ty) =
                            if let rustc_middle::ty::TyKind::Adt(adt_def, substs) = field_ty.kind()
                            {
                                if self.is_std_type(adt_def, "Option") {
                                    let inner_ty = substs[0].expect_ty();
                                    (true, self.convert_type(inner_ty, &field_context))
                                } else {
                                    (false, self.convert_type(field_ty, &field_context))
                                }
                            } else {
                                (false, self.convert_type(field_ty, &field_context))
                            };

                        TsField {
                            name: field_name.clone(),
                            ty: actual_ty,
                            is_optional,
                        }
                    })
                    .collect();

                let tag_field = TsField {
                    name: "t".to_string(),
                    ty: TsType::Primitive(format!("\"{}\"", variant.name)),
                    is_optional: false,
                };

                let value_field = TsField {
                    name: "v".to_string(),
                    ty: if is_tuple_variant && fields.len() == 1 {
                        fields[0].ty.clone()
                    } else if is_tuple_variant {
                        TsType::Tuple(fields.into_iter().map(|f| f.ty).collect())
                    } else {
                        TsType::Object(fields)
                    },
                    is_optional: false,
                };

                TsType::Object(vec![tag_field, value_field])
            };
            variants.push(variant_ty);
        }

        TsType::Union(variants)
    }
}

fn get_module_actual_span<'tcx>(tcx: TyCtxt<'tcx>, def_id: DefId) -> rustc_span::Span {
    if let Some(local_def_id) = def_id.as_local() {
        let hir_id = tcx.local_def_id_to_hir_id(local_def_id);
        if let rustc_hir::Node::Item(item) = tcx.hir_node(hir_id)
            && let rustc_hir::ItemKind::Mod(_, mod_ref) = &item.kind
            && let Some(first_item_id) = mod_ref.item_ids.first()
        {
            let first_item_hir_id = first_item_id.hir_id();
            if let rustc_hir::Node::Item(first_item) = tcx.hir_node(first_item_hir_id) {
                return first_item.span;
            }
        }
    }
    tcx.def_span(def_id)
}

fn strip_route_prefix(full_path: &str) -> Vec<String> {
    let parts: Vec<&str> = full_path.split("::").collect();

    // Find "pages" index
    if let Some(pages_idx) = parts.iter().position(|&p| p == "pages") {
        // Skip route_generated, pages, and the page module name
        let start_idx = pages_idx + 2;
        if start_idx < parts.len() {
            return parts[start_idx..].iter().map(|s| s.to_string()).collect();
        }
    }

    // Fallback: return just the last component
    vec![parts.last().unwrap_or(&"Unknown").to_string()]
}

/// Represents the resolved name for a type: namespace path + type name
#[derive(Debug, Clone)]
struct ResolvedName {
    /// ["utils"] or []
    namespace: Vec<String>,
    /// "Review"
    type_name: String,
}

impl ResolvedName {
    /// Get the full reference name for use in types (e.g., "utils.Review" or "Review")
    fn reference(&self) -> String {
        if self.namespace.is_empty() {
            self.type_name.clone()
        } else {
            format!("{}.{}", self.namespace.join("."), self.type_name)
        }
    }
}

fn find_shortest_unique_namespaces(paths: &[Vec<String>]) -> Vec<ResolvedName> {
    if paths.len() == 1 {
        // No collision - no namespace needed
        let type_name = paths[0].last().unwrap().clone();
        return vec![ResolvedName {
            namespace: vec![],
            type_name,
        }];
    }

    // There's a collision, need to find shortest unique prefix
    let mut suffix_len = 1;

    loop {
        let mut resolved: Vec<ResolvedName> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut has_collision = false;

        for components in paths.iter() {
            let type_name = components.last().unwrap().clone();

            // Try using suffix_len components as namespace
            let namespace_len = components.len().saturating_sub(1).min(suffix_len);
            let namespace_start = components.len().saturating_sub(1) - namespace_len;
            let namespace: Vec<String> = components[namespace_start..components.len() - 1]
                .iter()
                .map(|s| s.to_string())
                .collect();

            let resolved_name = ResolvedName {
                namespace: namespace.clone(),
                type_name: type_name.clone(),
            };

            let key = resolved_name.reference();
            if seen.contains_key(&key) {
                has_collision = true;
                break;
            }
            seen.insert(key, resolved.len());
            resolved.push(resolved_name);
        }

        if !has_collision {
            return resolved;
        }

        suffix_len += 1;

        // Safety: if we've used all available namespace components, stop
        if suffix_len
            > paths
                .iter()
                .map(|p| p.len().saturating_sub(1))
                .max()
                .unwrap_or(0)
        {
            // Use full namespace path
            return paths
                .iter()
                .map(|components| {
                    let type_name = components.last().unwrap().clone();
                    let namespace = components[..components.len() - 1]
                        .iter()
                        .map(|s| s.to_string())
                        .collect();
                    ResolvedName {
                        namespace,
                        type_name,
                    }
                })
                .collect();
        }
    }
}

fn resolve_type_names(definitions: &[TsDefinition]) -> HashMap<String, ResolvedName> {
    let mut groups: HashMap<String, Vec<(String, Vec<String>)>> = HashMap::new();

    for def in definitions {
        let components = strip_route_prefix(&def.full_path);
        let final_component = components.last().unwrap().clone();

        groups
            .entry(final_component)
            .or_default()
            .push((def.full_path.clone(), components));
    }

    let mut result = HashMap::new();

    for (_final_name, paths) in groups {
        let components_only: Vec<Vec<String>> = paths.iter().map(|(_, c)| c.clone()).collect();
        let resolved_names = find_shortest_unique_namespaces(&components_only);

        for ((full_path, _), resolved_name) in paths.iter().zip(resolved_names.iter()) {
            result.insert(full_path.clone(), resolved_name.clone());
        }
    }

    result
}

fn apply_name_resolution_to_type(ty: &mut TsType, name_map: &HashMap<String, ResolvedName>) {
    match ty {
        TsType::Reference(name) => {
            if let Some(resolved) = name_map.get(name.as_str()) {
                *name = resolved.reference();
            }
        }
        TsType::Array(inner) => {
            apply_name_resolution_to_type(inner, name_map);
        }
        TsType::Tuple(types) => {
            for t in types {
                apply_name_resolution_to_type(t, name_map);
            }
        }
        TsType::Union(types) => {
            for t in types {
                apply_name_resolution_to_type(t, name_map);
            }
        }
        TsType::Object(fields) => {
            for field in fields {
                apply_name_resolution_to_type(&mut field.ty, name_map);
            }
        }
        TsType::Undefined(inner) => {
            apply_name_resolution_to_type(inner, name_map);
        }
        TsType::Primitive(_) => {}
    }
}

fn format_definition(def: &TsDefinition) -> String {
    if let TsType::Object(fields) = &def.ty {
        let mut result = format!("export interface {} {{\n", def.type_name);
        for field in fields {
            let optional_marker = if field.is_optional { "?" } else { "" };
            let ty_str = if field.is_optional {
                strip_undefined(&field.ty)
            } else {
                format!("{}", field.ty)
            };
            result.push_str(&format!(
                "    {}{}: {};\n",
                field.name, optional_marker, ty_str
            ));
        }
        result.push('}');
        result
    } else {
        format!("export type {} = {};", def.type_name, def.ty)
    }
}

struct Analyzer {
    ts_output_dir: String,
}

fn convert_rust_path_to_ts_path(rust_path: &str, ts_output_dir: &str) -> PathBuf {
    let path_str = rust_path.to_string();
    let path_parts: Vec<&str> = path_str.split('/').collect();
    let src_pages_idx = path_parts.iter().position(|&p| p == "pages");

    if let Some(idx) = src_pages_idx {
        let after_pages = &path_parts[idx + 1..path_parts.len() - 1];
        let relative_path = after_pages.join("/");

        let mut output_path = PathBuf::from(ts_output_dir);
        output_path.push(relative_path);
        output_path.push(".props.ts");

        output_path
    } else {
        PathBuf::from(ts_output_dir)
    }
}

impl Callbacks for Analyzer {
    fn after_analysis<'tcx>(&mut self, _compiler: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let items = tcx.hir_crate_items(());
        let page_modules = Mutex::new(Vec::new());
        let _ = items.par_items(|item_id| {
            let owner_id = item_id.owner_id;
            let def_id: DefId = owner_id.to_def_id();
            if tcx.def_kind(def_id) == DefKind::Mod {
                let span = get_module_actual_span(tcx, def_id);
                let source_map = tcx.sess.source_map();
                let filename = source_map.span_to_filename(span);
                if let rustc_span::FileName::Real(path) = filename
                    && let Some(local_path) = path.into_local_path()
                {
                    let path_str = local_path.to_string_lossy();
                    if path_str.contains("src/pages") && path_str.ends_with("mod.rs") {
                        let path_parts: Vec<&str> = path_str.split('/').collect();
                        let src_pages_idx = path_parts.iter().position(|&p| p == "pages");
                        if let Some(idx) = src_pages_idx {
                            let after_pages = &path_parts[idx + 1..path_parts.len() - 1];
                            if after_pages.len() <= 2 {
                                let mut modules = page_modules.lock().unwrap();
                                modules.push(def_id);
                            }
                        }
                    }
                }
            }
            Ok(())
        });
        let source_map = tcx.sess.source_map();
        let modules = page_modules.lock().unwrap();
        for def_id in modules.iter() {
            let def_id = *def_id;
            let span = get_module_actual_span(tcx, def_id);
            let filename = source_map.span_to_filename(span);
            if let rustc_span::FileName::Real(ref path) = filename
                && let Some(local_path) = path.clone().into_local_path()
            {
                let path_str = local_path.to_string_lossy();
                println!("Found: {}", path_str);
            }
            let local_def_id = match def_id.as_local() {
                Some(id) => id,
                None => continue,
            };
            let hir_id = tcx.local_def_id_to_hir_id(local_def_id);
            let filename = source_map.span_to_filename(span);
            let mut handler_found = false;
            let mut props_def_id: Option<DefId> = None;
            let mut props_kind: Option<DefKind> = None;

            if let rustc_hir::Node::Item(item) = tcx.hir_node(hir_id)
                && let rustc_hir::ItemKind::Mod(_, mod_ref) = &item.kind
            {
                for item_id in mod_ref.item_ids {
                    let item_hir_id = item_id.hir_id();
                    if let rustc_hir::Node::Item(child_item) = tcx.hir_node(item_hir_id) {
                        let child_def_id = child_item.owner_id.to_def_id();
                        let item_name_str = tcx.def_path_str(child_def_id);
                        if let Some((_, name)) = item_name_str.rsplit_once("::") {
                            if name == "handler" {
                                if tcx.def_kind(child_def_id) == DefKind::Fn {
                                    if matches!(tcx.visibility(child_def_id), Visibility::Public) {
                                        handler_found = true;
                                    } else {
                                        let path_str = if let rustc_span::FileName::Real(ref path) =
                                            filename
                                        {
                                            path.clone()
                                                .into_local_path()
                                                .map(|p| p.to_string_lossy().to_string())
                                                .unwrap_or_else(|| format!("{:?}", filename))
                                        } else {
                                            format!("{:?}", filename)
                                        };
                                        eprintln!("Error: handler in {} must be public", path_str);
                                        std::process::exit(1);
                                    }
                                }
                            } else if name == "Props" {
                                let item_def_kind = tcx.def_kind(child_def_id);
                                if item_def_kind == DefKind::Struct
                                    || item_def_kind == DefKind::Enum
                                    || item_def_kind == DefKind::TyAlias
                                {
                                    props_def_id = Some(child_def_id);
                                    props_kind = Some(item_def_kind);
                                } else {
                                    let path_str =
                                        if let rustc_span::FileName::Real(ref path) = filename {
                                            path.clone()
                                                .into_local_path()
                                                .map(|p| p.to_string_lossy().to_string())
                                                .unwrap_or_else(|| format!("{:?}", filename))
                                        } else {
                                            format!("{:?}", filename)
                                        };
                                    eprintln!(
                                        "Error: Props in {} must be a struct, enum, or type alias",
                                        path_str
                                    );
                                    std::process::exit(1);
                                }
                            }
                        }
                    }
                }
            }
            if !handler_found {
                let span = get_module_actual_span(tcx, def_id);
                let filename = source_map.span_to_filename(span);
                let path_str = if let rustc_span::FileName::Real(ref path) = filename {
                    path.clone()
                        .into_local_path()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| format!("{:?}", filename))
                } else {
                    format!("{:?}", filename)
                };
                eprintln!("Error: handler not found or not public in {}", path_str);
                std::process::exit(1);
            }
            if let Some(props_id) = props_def_id {
                let kind_name = match props_kind {
                    Some(DefKind::Struct) => "struct",
                    Some(DefKind::Enum) => "enum",
                    Some(DefKind::TyAlias) => "alias",
                    _ => "unknown",
                };
                let span = get_module_actual_span(tcx, def_id);
                let filename = source_map.span_to_filename(span);
                let rust_source_path = if let rustc_span::FileName::Real(ref path) = filename
                    && let Some(local_path) = path.clone().into_local_path()
                {
                    local_path.to_string_lossy().to_string()
                } else {
                    format!("{:?}", filename)
                };

                println!("{} -> Props ({})", rust_source_path, kind_name);

                let mut converter = TypeConverter::new(tcx);
                let props_ty = tcx.type_of(props_id).instantiate_identity();
                let context = format!("{:?}", filename);
                let ts_type = converter.convert_type(props_ty, &context);

                let name_map = resolve_type_names(&converter.definitions);

                for def in &mut converter.definitions {
                    if let Some(resolved) = name_map.get(&def.full_path) {
                        def.namespace = resolved.namespace.clone();
                        def.type_name = resolved.type_name.clone();
                    }
                }

                for def in &mut converter.definitions {
                    apply_name_resolution_to_type(&mut def.ty, &name_map);
                }

                let mut file_content = String::new();
                file_content.push_str(&format!("// Auto-generated from {}\n\n", rust_source_path));

                if let TsType::Object(fields) = &ts_type {
                    file_content.push_str("export interface Props {\n");
                    for field in fields {
                        let optional_marker = if field.is_optional { "?" } else { "" };
                        let ty_str = if field.is_optional {
                            strip_undefined(&field.ty)
                        } else {
                            format!("{}", field.ty)
                        };
                        file_content.push_str(&format!(
                            "    {}{}: {};\n",
                            field.name, optional_marker, ty_str
                        ));
                    }
                    file_content.push_str("}\n");
                } else {
                    file_content.push_str(&format!("export type Props = {};\n", ts_type));
                }

                let mut namespace_groups: HashMap<Vec<String>, Vec<&TsDefinition>> = HashMap::new();
                for def in &converter.definitions {
                    namespace_groups
                        .entry(def.namespace.clone())
                        .or_default()
                        .push(def);
                }

                if let Some(top_level_defs) = namespace_groups.get(&vec![]) {
                    for def in top_level_defs {
                        file_content.push('\n');
                        file_content.push_str(&format_definition(def));
                    }
                }

                let mut namespaces: Vec<Vec<String>> = namespace_groups
                    .keys()
                    .filter(|ns| !ns.is_empty())
                    .cloned()
                    .collect();
                namespaces.sort();

                for namespace in namespaces {
                    file_content.push('\n');
                    file_content
                        .push_str(&format!("export namespace {} {{\n", namespace.join(".")));

                    if let Some(defs) = namespace_groups.get(&namespace) {
                        for def in defs {
                            let def_str = format_definition(def);
                            // Indent each line
                            for line in def_str.lines() {
                                file_content.push_str(&format!("    {}\n", line));
                            }
                        }
                    }

                    file_content.push_str("}\n");
                }

                let ts_output_path =
                    convert_rust_path_to_ts_path(&rust_source_path, &self.ts_output_dir);

                if let Some(parent) = ts_output_path.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    eprintln!("Error creating directory {}: {}", parent.display(), e);
                    std::process::exit(1);
                }

                if let Err(e) = std::fs::write(&ts_output_path, &file_content) {
                    eprintln!("Error writing file {}: {}", ts_output_path.display(), e);
                    std::process::exit(1);
                }

                println!(
                    "Generated: {} -> {}",
                    rust_source_path,
                    ts_output_path.display()
                );
            } else {
                let span = get_module_actual_span(tcx, def_id);
                let filename = source_map.span_to_filename(span);
                let path_str = if let rustc_span::FileName::Real(ref path) = filename {
                    path.clone()
                        .into_local_path()
                        .map(|p| p.to_string_lossy().to_string())
                        .unwrap_or_else(|| format!("{:?}", filename))
                } else {
                    format!("{:?}", filename)
                };
                eprintln!("Error: Props not found in {}", path_str);
                std::process::exit(1);
            }
        }
        Compilation::Stop
    }
}
fn main() {
    if env::var("MY_ANALYZER_WRAPPER_MODE").is_ok() {
        let ts_output_dir =
            env::var("TS_OUTPUT_DIR").unwrap_or_else(|_| "../fe/src/pages".to_string());

        let mut args: Vec<String> = env::args().collect();

        let is_build_script = args.iter().any(|arg| arg == "build_script_build");

        if is_build_script {
            let rustc_path = &args[1];
            let rustc_args = &args[2..];

            let status = Command::new(rustc_path)
                .args(rustc_args)
                .status()
                .expect("Failed to execute original rustc");

            std::process::exit(status.code().unwrap_or(1));
        }

        if args.len() > 1 {
            args.remove(1);
        }
        let mut callbacks = Analyzer { ts_output_dir };
        run_compiler(&args, &mut callbacks);
        return;
    }
    let target_dir = env::args()
        .nth(1)
        .unwrap_or_else(|| "../forte-manual/rs".to_string());

    let ts_output_dir = env::args().nth(2).unwrap_or_else(|| {
        let parent = Path::new(&target_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        format!("{}/fe/src/pages", parent)
    });

    let current_exe = env::current_exe().expect("Failed to find current exe");
    println!("Running cargo check on: {target_dir}");
    println!("Writing TypeScript to: {ts_output_dir}");

    let status = Command::new("cargo")
        .arg("check")
        .current_dir(&target_dir)
        .env("RUSTC_WORKSPACE_WRAPPER", current_exe)
        .env("MY_ANALYZER_WRAPPER_MODE", "true")
        .env("TS_OUTPUT_DIR", &ts_output_dir)
        .status()
        .expect("Failed to run cargo");

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
}
