/// Intermediate representation of parsed Rust types
/// This serves as a bridge between Rust AST and TypeScript generation

#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub structs: Vec<ParsedStruct>,
    pub enums: Vec<ParsedEnum>,
}

#[derive(Debug, Clone)]
pub struct ParsedStruct {
    pub name: String,
    pub fields: Vec<ParsedField>,
    pub is_page_props: bool,
    pub is_action_input: bool,
    pub has_validate: bool,  // Whether this struct has #[derive(Validate)]
}

#[derive(Debug, Clone)]
pub struct ParsedField {
    pub name: String,
    pub renamed_to: Option<String>, // from #[serde(rename = "...")]
    pub skip: bool,                 // from #[serde(skip)]
    pub field_type: ParsedType,
}

#[derive(Debug, Clone)]
pub struct ParsedEnum {
    pub name: String,
    pub variants: Vec<ParsedVariant>,
    pub tag: Option<String>, // from #[serde(tag = "...")]
}

#[derive(Debug, Clone)]
pub struct ParsedVariant {
    pub name: String,
    pub fields: Option<Vec<ParsedField>>, // None for unit variants, Some for struct variants
}

#[derive(Debug, Clone)]
pub enum ParsedType {
    String,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Option(Box<ParsedType>),
    Vec(Box<ParsedType>),
    HashMap(Box<ParsedType>, Box<ParsedType>),
    Custom(String), // For custom types like User, Product, etc.
}

impl ParsedField {
    /// Get the effective field name (renamed or original)
    pub fn effective_name(&self) -> &str {
        self.renamed_to.as_deref().unwrap_or(&self.name)
    }
}
