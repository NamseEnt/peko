User pass rust project path to this program.

This program analyzes Rust code that,

1. Finds `rs/src/pages/**/mod.rs`
2. Filters only those with `pub fn handler`
3. That mod must have `Props` type defined
4. Extracts the `Props` type definition recursively
5. Converts Rust type definitions to TypeScript equivalents
6. Outputs the TypeScript definitions to ts project.
   - For example, `rs/src/pages/foo/mod.rs` -> `ts/pages/foo/.props.ts`
   - `rs/src/pages/product/[id]/mod.rs` -> `ts/pages/product/[id]/.props.ts`
   - All interfaces are exported like `export interface Bar` in ts files.

For example, check ../forte-manual directory. there are rs and ts directories.
You have to test this program with ../forte-manual directory.
