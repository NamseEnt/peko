use colored::*;
use serde::Deserialize;

/// Rust compiler diagnostic message
#[derive(Debug, Deserialize)]
pub struct CompilerMessage {
    pub message: String,
    pub code: Option<ErrorCode>,
    pub level: String,
    pub spans: Vec<DiagnosticSpan>,
    pub children: Vec<CompilerMessage>,
    pub rendered: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorCode {
    pub code: String,
}

#[derive(Debug, Deserialize)]
pub struct DiagnosticSpan {
    pub file_name: String,
    pub line_start: usize,
    pub line_end: usize,
    pub column_start: usize,
    pub column_end: usize,
    pub is_primary: bool,
    pub label: Option<String>,
}

/// Parse Rust compiler JSON output and format it nicely
pub fn format_compiler_errors(stderr: &str) -> String {
    let mut formatted_errors = Vec::new();

    for line in stderr.lines() {
        // Skip non-JSON lines
        if !line.trim().starts_with('{') {
            continue;
        }

        // Try to parse as compiler message
        if let Ok(msg) = serde_json::from_str::<CompilerMessage>(line) {
            if msg.level == "error" {
                formatted_errors.push(format_error(&msg));
            }
        }
    }

    if formatted_errors.is_empty() {
        // If no structured errors found, return raw stderr
        return stderr.to_string();
    }

    formatted_errors.join("\n\n")
}

fn format_error(msg: &CompilerMessage) -> String {
    let mut output = String::new();

    // Top border
    output.push_str(&format!("\n{}\n", "┌─ Compilation Error ──────────────────────────────┐".red()));
    output.push_str(&format!("{}\n", "│".red()));

    // Find primary span for location
    if let Some(span) = msg.spans.iter().find(|s| s.is_primary) {
        let location = format!(
            "  {}:{}:{}",
            span.file_name,
            span.line_start,
            span.column_start
        );
        output.push_str(&format!("{}  {:<48}{}\n", "│".red(), location.yellow(), "│".red()));
        output.push_str(&format!("{}\n", "│".red()));
    }

    // Error code
    if let Some(code) = &msg.code {
        let error_line = format!("  error[{}]: {}", code.code, msg.message);
        for chunk in error_line.chars().collect::<Vec<_>>().chunks(48) {
            let chunk_str: String = chunk.iter().collect();
            output.push_str(&format!("{}  {:<48}{}\n", "│".red(), chunk_str.bold(), "│".red()));
        }
    } else {
        let error_line = format!("  error: {}", msg.message);
        for chunk in error_line.chars().collect::<Vec<_>>().chunks(48) {
            let chunk_str: String = chunk.iter().collect();
            output.push_str(&format!("{}  {:<48}{}\n", "│".red(), chunk_str.bold(), "│".red()));
        }
    }

    // Show labels from children
    for child in &msg.children {
        if let Some(span) = child.spans.first() {
            if let Some(label) = &span.label {
                let label_line = format!("    {}", label);
                for chunk in label_line.chars().collect::<Vec<_>>().chunks(48) {
                    let chunk_str: String = chunk.iter().collect();
                    output.push_str(&format!("{}  {:<48}{}\n", "│".red(), chunk_str.dimmed(), "│".red()));
                }
            }
        } else {
            let child_msg = format!("    {}", child.message);
            for chunk in child_msg.chars().collect::<Vec<_>>().chunks(48) {
                let chunk_str: String = chunk.iter().collect();
                output.push_str(&format!("{}  {:<48}{}\n", "│".red(), chunk_str.dimmed(), "│".red()));
            }
        }
    }

    // Bottom border
    output.push_str(&format!("{}\n", "│".red()));
    output.push_str(&format!("{}", "└──────────────────────────────────────────────────┘".red()));

    output
}
