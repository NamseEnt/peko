use crate::parser::types::*;
use heck::ToLowerCamelCase;

/// Generate TypeScript code from parsed Rust types
pub fn generate_typescript(parsed: &ParsedFile) -> String {
    let mut output = String::new();

    // Header comment
    output.push_str("// [Generated] Do not edit manually\n");
    output.push_str("// This file is auto-generated from backend/src/routes/**/props.rs\n\n");

    // Generate interfaces for structs
    for struct_def in &parsed.structs {
        generate_struct(&mut output, struct_def);
        output.push('\n');
    }

    // Generate types for enums
    for enum_def in &parsed.enums {
        generate_enum(&mut output, enum_def);
        output.push('\n');
    }

    output
}

fn generate_struct(output: &mut String, struct_def: &ParsedStruct) {
    output.push_str(&format!("export interface {} {{\n", struct_def.name));

    for field in &struct_def.fields {
        if field.skip {
            continue;
        }

        let field_name = field.effective_name();
        let ts_type = type_to_typescript(&field.field_type);
        output.push_str(&format!("  {}: {};\n", field_name, ts_type));
    }

    output.push_str("}\n");
}

fn generate_enum(output: &mut String, enum_def: &ParsedEnum) {
    // Handle empty enums (no variants)
    if enum_def.variants.is_empty() {
        output.push_str(&format!("// {} is empty - add variants as needed\n", enum_def.name));
        output.push_str(&format!("export type {} = never;\n", enum_def.name));
        return;
    }

    output.push_str(&format!("export type {} =\n", enum_def.name));

    for (i, variant) in enum_def.variants.iter().enumerate() {
        let separator = if i == 0 { "  " } else { "| " };

        if let Some(tag) = &enum_def.tag {
            // Tagged union (discriminated union)
            if let Some(fields) = &variant.fields {
                output.push_str(&format!("  {} {{ {}: \"{}\"", separator, tag, variant.name));
                for field in fields {
                    let field_name = field.effective_name();
                    let ts_type = type_to_typescript(&field.field_type);
                    output.push_str(&format!("; {}: {}", field_name, ts_type));
                }
                output.push_str(" }\n");
            } else {
                output.push_str(&format!(
                    "  {} {{ {}: \"{}\" }}\n",
                    separator, tag, variant.name
                ));
            }
        } else {
            // Untagged enum
            if let Some(fields) = &variant.fields {
                output.push_str(&format!("  {} {{ kind: \"{}\"", separator, variant.name));
                for field in fields {
                    let field_name = field.effective_name();
                    let ts_type = type_to_typescript(&field.field_type);
                    output.push_str(&format!("; {}: {}", field_name, ts_type));
                }
                output.push_str(" }\n");
            } else {
                output.push_str(&format!("  {} \"{}\"\n", separator, variant.name));
            }
        }
    }

    output.push_str(";\n");
}

fn type_to_typescript(parsed_type: &ParsedType) -> String {
    match parsed_type {
        ParsedType::String => "string".to_string(),
        ParsedType::Bool => "boolean".to_string(),
        ParsedType::I8
        | ParsedType::I16
        | ParsedType::I32
        | ParsedType::I64
        | ParsedType::U8
        | ParsedType::U16
        | ParsedType::U32
        | ParsedType::U64
        | ParsedType::F32
        | ParsedType::F64 => "number".to_string(),
        ParsedType::Option(inner) => {
            format!("{} | null", type_to_typescript(inner))
        }
        ParsedType::Vec(inner) => {
            format!("{}[]", type_to_typescript(inner))
        }
        ParsedType::HashMap(key, value) => {
            // Assuming key is always string for now
            format!("Record<{}, {}>", type_to_typescript(key), type_to_typescript(value))
        }
        ParsedType::Custom(name) => name.clone(),
    }
}

/// Convert snake_case to camelCase (currently unused, reserved for future)
#[allow(dead_code)]
pub fn to_camel_case(s: &str) -> String {
    s.to_lower_camel_case()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_struct() {
        let parsed = ParsedFile {
            structs: vec![ParsedStruct {
                name: "PageProps".to_string(),
                fields: vec![
                    ParsedField {
                        name: "message".to_string(),
                        renamed_to: None,
                        skip: false,
                        field_type: ParsedType::String,
                    },
                    ParsedField {
                        name: "count".to_string(),
                        renamed_to: None,
                        skip: false,
                        field_type: ParsedType::I32,
                    },
                ],
                is_page_props: true,
                is_action_input: false,
                has_validate: false,
            }],
            enums: vec![],
        };

        let output = generate_typescript(&parsed);
        assert!(output.contains("export interface PageProps"));
        assert!(output.contains("message: string;"));
        assert!(output.contains("count: number;"));
    }

    #[test]
    fn test_option_type() {
        let field_type = ParsedType::Option(Box::new(ParsedType::String));
        let ts_type = type_to_typescript(&field_type);
        assert_eq!(ts_type, "string | null");
    }

    #[test]
    fn test_vec_type() {
        let field_type = ParsedType::Vec(Box::new(ParsedType::I32));
        let ts_type = type_to_typescript(&field_type);
        assert_eq!(ts_type, "number[]");
    }
}
