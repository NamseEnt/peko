use anyhow::{Context, Result};
use std::fs;
use syn::{
    Attribute, Fields, GenericArgument, PathArguments,
    Type, TypePath,
};

use super::types::*;

/// Parse a props.rs file and extract all type definitions
pub fn parse_props_file(path: &str) -> Result<ParsedFile> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read file: {}", path))?;

    let ast = syn::parse_file(&content)
        .with_context(|| format!("Failed to parse Rust file: {}", path))?;

    let mut parsed_file = ParsedFile {
        structs: Vec::new(),
        enums: Vec::new(),
    };

    for item in ast.items {
        match item {
            syn::Item::Struct(item_struct) => {
                if let Some(parsed_struct) = parse_struct(item_struct)? {
                    parsed_file.structs.push(parsed_struct);
                }
            }
            syn::Item::Enum(item_enum) => {
                if let Some(parsed_enum) = parse_enum(item_enum)? {
                    parsed_file.enums.push(parsed_enum);
                }
            }
            _ => {}
        }
    }

    Ok(parsed_file)
}

fn parse_struct(item: syn::ItemStruct) -> Result<Option<ParsedStruct>> {
    // Only parse public structs
    if !matches!(item.vis, syn::Visibility::Public(_)) {
        return Ok(None);
    }

    let name = item.ident.to_string();
    let is_page_props = name == "PageProps";
    let is_action_input = name == "ActionInput";

    // Check if struct has #[derive(Validate)]
    let has_validate = has_derive_validate(&item.attrs);

    let fields = match item.fields {
        Fields::Named(fields) => {
            let mut parsed_fields = Vec::new();
            for field in fields.named {
                if let Some(parsed_field) = parse_field(field)? {
                    parsed_fields.push(parsed_field);
                }
            }
            parsed_fields
        }
        _ => Vec::new(),
    };

    Ok(Some(ParsedStruct {
        name,
        fields,
        is_page_props,
        is_action_input,
        has_validate,
    }))
}

/// Check if attributes contain #[derive(Validate)]
fn has_derive_validate(attrs: &[Attribute]) -> bool {
    for attr in attrs {
        if attr.path().is_ident("derive") {
            // Parse as Meta to extract derive arguments
            if let syn::Meta::List(meta_list) = &attr.meta {
                let tokens = meta_list.tokens.to_string();
                if tokens.contains("Validate") {
                    return true;
                }
            }
        }
    }
    false
}

fn parse_field(field: syn::Field) -> Result<Option<ParsedField>> {
    // Only parse public fields
    if !matches!(field.vis, syn::Visibility::Public(_)) {
        return Ok(None);
    }

    let name = field
        .ident
        .map(|i| i.to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let (renamed_to, skip) = parse_serde_attrs(&field.attrs)?;

    if skip {
        return Ok(None);
    }

    let field_type = parse_type(&field.ty)?;

    Ok(Some(ParsedField {
        name,
        renamed_to,
        skip,
        field_type,
    }))
}

fn parse_enum(item: syn::ItemEnum) -> Result<Option<ParsedEnum>> {
    // Only parse public enums
    if !matches!(item.vis, syn::Visibility::Public(_)) {
        return Ok(None);
    }

    let name = item.ident.to_string();
    let tag = parse_serde_tag(&item.attrs)?;

    let mut variants = Vec::new();
    for variant in item.variants {
        let variant_name = variant.ident.to_string();
        let fields = match variant.fields {
            Fields::Named(fields) => {
                let mut parsed_fields = Vec::new();
                for field in fields.named {
                    // Enum variant fields don't have visibility, so parse them differently
                    let name = field
                        .ident
                        .map(|i| i.to_string())
                        .unwrap_or_else(|| "unknown".to_string());

                    let (renamed_to, skip) = parse_serde_attrs(&field.attrs)?;

                    if skip {
                        continue;
                    }

                    let field_type = parse_type(&field.ty)?;

                    parsed_fields.push(ParsedField {
                        name,
                        renamed_to,
                        skip,
                        field_type,
                    });
                }
                Some(parsed_fields)
            }
            Fields::Unit => None,
            _ => None,
        };

        variants.push(ParsedVariant {
            name: variant_name,
            fields,
        });
    }

    Ok(Some(ParsedEnum {
        name,
        variants,
        tag,
    }))
}

fn parse_type(ty: &Type) -> Result<ParsedType> {
    match ty {
        Type::Path(TypePath { path, .. }) => {
            let segment = path
                .segments
                .last()
                .context("Type path has no segments")?;

            let type_name = segment.ident.to_string();

            match type_name.as_str() {
                "String" | "str" => Ok(ParsedType::String),
                "bool" => Ok(ParsedType::Bool),
                "i8" => Ok(ParsedType::I8),
                "i16" => Ok(ParsedType::I16),
                "i32" => Ok(ParsedType::I32),
                "i64" => Ok(ParsedType::I64),
                "u8" => Ok(ParsedType::U8),
                "u16" => Ok(ParsedType::U16),
                "u32" => Ok(ParsedType::U32),
                "u64" => Ok(ParsedType::U64),
                "f32" => Ok(ParsedType::F32),
                "f64" => Ok(ParsedType::F64),
                "Option" => {
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(GenericArgument::Type(inner_type)) = args.args.first() {
                            let inner = parse_type(inner_type)?;
                            return Ok(ParsedType::Option(Box::new(inner)));
                        }
                    }
                    anyhow::bail!("Option type without generic argument")
                }
                "Vec" => {
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        if let Some(GenericArgument::Type(inner_type)) = args.args.first() {
                            let inner = parse_type(inner_type)?;
                            return Ok(ParsedType::Vec(Box::new(inner)));
                        }
                    }
                    anyhow::bail!("Vec type without generic argument")
                }
                "HashMap" => {
                    if let PathArguments::AngleBracketed(args) = &segment.arguments {
                        let mut args_iter = args.args.iter();
                        if let (
                            Some(GenericArgument::Type(key_type)),
                            Some(GenericArgument::Type(value_type)),
                        ) = (args_iter.next(), args_iter.next())
                        {
                            let key = parse_type(key_type)?;
                            let value = parse_type(value_type)?;
                            return Ok(ParsedType::HashMap(Box::new(key), Box::new(value)));
                        }
                    }
                    anyhow::bail!("HashMap type without generic arguments")
                }
                _ => Ok(ParsedType::Custom(type_name)),
            }
        }
        _ => anyhow::bail!("Unsupported type: {:?}", ty),
    }
}

fn parse_serde_attrs(attrs: &[Attribute]) -> Result<(Option<String>, bool)> {
    let mut renamed_to = None;
    let mut skip = false;

    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }

        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                renamed_to = Some(s.value());
                Ok(())
            } else if meta.path.is_ident("skip") {
                skip = true;
                Ok(())
            } else {
                Ok(())
            }
        })?;
    }

    Ok((renamed_to, skip))
}

fn parse_serde_tag(attrs: &[Attribute]) -> Result<Option<String>> {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }

        let mut tag = None;
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("tag") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                tag = Some(s.value());
            }
            Ok(())
        })?;

        if tag.is_some() {
            return Ok(tag);
        }
    }

    Ok(None)
}
