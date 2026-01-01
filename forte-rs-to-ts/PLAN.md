### Plan Overview

The goal is to transition the program from printing TypeScript definitions to the console (stdout) to writing them into specific files within a target TypeScript project directory. We need to calculate the correct destination path based on the source Rust file path and handle file system operations.

---

### Step 1: Update Command Line Arguments

Modify the `main` function to accept an optional second argument for the **TypeScript output directory**.

1.  Currently, the program accepts `target_dir` (the Rust project path).
2.  Add logic to accept a second argument (e.g., `ts_output_dir`).
3.  If the user does not provide it, define a sensible default (e.g., `../ts` relative to the Rust project) or derive it from the `rs` path.
4.  Pass this output path context into the `Analyzer` struct so it is accessible during the analysis phase.

### Step 2: Implement Path Conversion Logic

Create a helper function to convert the Rust source path to the TypeScript destination path.

1.  **Input:** The full path of the Rust file (e.g., `/.../rs/src/pages/product/[id]/mod.rs`).
2.  **Logic:**
    - Identify the segment `src/pages`.
    - Extract the relative path after `src` (e.g., `pages/product/[id]`).
    - Remove the `mod.rs` filename.
    - Append `.props.ts` to the directory path.
    - Join this with the `ts_output_dir` determined in Step 1.
3.  **Output:** The full destination path (e.g., `/.../ts/pages/product/[id]/.props.ts`).

### Step 3: Refactor Output Generation (Buffer vs Print)

Modify the `Analyzer::after_analysis` method to collect the TypeScript code into a string buffer instead of printing to stdout.

1.  Identify the section where `println!` is currently used to output `export interface Props` and other definitions.
2.  Initialize a `String` buffer (e.g., `let mut file_content = String::new();`).
3.  Add a header comment to the buffer indicating the file is auto-generated (e.g., `// Auto-generated from rustc`).
4.  Write the `Props` definition (either interface or type alias) into the buffer.
5.  Iterate through `converter.definitions` and append all dependent interfaces/types to the buffer.
6.  Ensure consistent formatting (newlines and indentation) within the string.

### Step 4: Implement File System Operations

Write the buffered content to the disk.

1.  Inside the loop where the `mod.rs` is processed:
    - Call the path conversion helper (Step 2) to get the `dest_path`.
    - Extract the parent directory of `dest_path`.
2.  Use `std::fs::create_dir_all` to ensure the directory structure exists (e.g., creating `ts/pages/product/[id]` if it doesn't exist).
3.  Use `std::fs::write` to save the `file_content` buffer to `dest_path`.
4.  Handle standard I/O errors gracefully (e.g., print an error to stderr if permission is denied, but don't crash the whole analyzer).

### Step 5: Clean Up and Feedback

Update the logging to reflect file generation.

1.  Remove the existing `println!` statements that dump the code to the terminal.
2.  Add a log message indicating success: `Generated: [Source Path] -> [Dest Path]`.
3.  Ensure error messages (like "handler not found") are still printed to stderr for debugging purposes.

### Step 6: Verify Edge Cases

Ensure the logic handles specific path scenarios mentioned in the goal:

1.  **Standard structure:** `rs/src/pages/foo/mod.rs` must map to `ts/pages/foo/.props.ts`.
2.  **Dynamic routes:** `rs/src/pages/product/[id]/mod.rs` must preserve the brackets in the folder name and map to `ts/pages/product/[id]/.props.ts`.
3.  **OS Compatibility:** Ensure path separators (`/` vs `\`) are handled correctly using `Path` and `PathBuf` primitives.
