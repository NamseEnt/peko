use crate::name_resolution::{apply_name_resolution_to_type, resolve_type_names};
use crate::rust_to_ts::TypeConverter;
use crate::ts_codegen::{TsDefinition, TsType, format_definition, strip_undefined};
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::def::DefKind;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::{TyCtxt, Visibility};
use rustc_span::def_id::DefId;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Analyzer {
    pub ts_output_dir: String,
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

fn generate_ts_file_content(
    rust_source_path: &str,
    ts_type: &TsType,
    definitions: &[TsDefinition],
) -> String {
    let mut file_content = String::new();
    file_content.push_str(&format!("// Auto-generated from {}\n\n", rust_source_path));

    if let TsType::Object(fields) = ts_type {
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
    for def in definitions {
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
        file_content.push_str(&format!("export namespace {} {{\n", namespace.join(".")));

        if let Some(defs) = namespace_groups.get(&namespace) {
            for def in defs {
                let def_str = format_definition(def);
                for line in def_str.lines() {
                    file_content.push_str(&format!("    {}\n", line));
                }
            }
        }

        file_content.push_str("}\n");
    }

    file_content
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

                let file_content =
                    generate_ts_file_content(&rust_source_path, &ts_type, &converter.definitions);

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
