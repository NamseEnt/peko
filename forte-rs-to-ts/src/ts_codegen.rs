use std::fmt;

#[derive(Debug, Clone)]
pub enum TsType {
    Primitive(String),
    Array(Box<TsType>),
    Tuple(Vec<TsType>),
    Union(Vec<TsType>),
    Object(Vec<TsField>),
    Undefined(Box<TsType>),
    Reference(String),
}

#[derive(Debug, Clone)]
pub struct TsField {
    pub name: String,
    pub ty: TsType,
    pub is_optional: bool,
}

pub fn strip_undefined(ty: &TsType) -> String {
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

#[derive(Debug, Clone)]
pub struct TsDefinition {
    pub full_path: String,
    pub namespace: Vec<String>,
    pub type_name: String,
    pub ty: TsType,
}

pub fn format_definition(def: &TsDefinition) -> String {
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
