use crate::ts_codegen::{TsDefinition, TsType};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ResolvedName {
    pub namespace: Vec<String>,
    pub type_name: String,
}

impl ResolvedName {
    pub fn reference(&self) -> String {
        if self.namespace.is_empty() {
            self.type_name.clone()
        } else {
            format!("{}.{}", self.namespace.join("."), self.type_name)
        }
    }
}

pub fn strip_route_prefix(full_path: &str) -> Vec<String> {
    let parts: Vec<&str> = full_path.split("::").collect();

    if let Some(pages_idx) = parts.iter().position(|&p| p == "pages") {
        let start_idx = pages_idx + 2;
        if start_idx < parts.len() {
            return parts[start_idx..].iter().map(|s| s.to_string()).collect();
        }
    }

    vec![parts.last().expect("Failed to get last part").to_string()]
}

pub fn find_shortest_unique_namespaces(paths: &[Vec<String>]) -> Vec<ResolvedName> {
    let no_collision = paths.len() == 1;
    if no_collision {
        let type_name = paths[0].last().unwrap().clone();
        return vec![ResolvedName {
            namespace: vec![],
            type_name,
        }];
    }

    let mut suffix_len = 1;

    loop {
        let mut resolved: Vec<ResolvedName> = Vec::new();
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut has_collision = false;

        for components in paths.iter() {
            let type_name = components.last().unwrap().clone();

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

        if suffix_len
            > paths
                .iter()
                .map(|p| p.len().saturating_sub(1))
                .max()
                .unwrap_or(0)
        {
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

pub fn resolve_type_names(definitions: &[TsDefinition]) -> HashMap<String, ResolvedName> {
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

pub fn apply_name_resolution_to_type(ty: &mut TsType, name_map: &HashMap<String, ResolvedName>) {
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
