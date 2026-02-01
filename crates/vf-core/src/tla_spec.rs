//! Basic TLA+ specification parser.
//!
//! Extracts invariants, line numbers, and evaluator mappings from TLA+ specs.
//! This is a simple parser for our specific format, not a full TLA+ parser.

use std::collections::HashMap;
use std::path::Path;

/// An invariant extracted from a TLA+ spec.
#[derive(Debug, Clone)]
pub struct TlaInvariant {
    /// Name of the invariant (e.g., "NoLostElements")
    pub name: String,
    /// Line number in the TLA+ file
    pub line: u32,
    /// Description if available
    pub description: Option<String>,
    /// Which evaluators should check this invariant
    pub evaluators: Vec<String>,
}

/// A parsed TLA+ specification.
#[derive(Debug, Clone)]
pub struct TlaSpec {
    /// Module name
    pub name: String,
    /// Full content of the spec
    pub content: String,
    /// Path to the spec file
    pub path: Option<String>,
    /// Constants defined in the spec
    pub constants: Vec<String>,
    /// Variables defined in the spec
    pub variables: Vec<String>,
    /// Invariants with line mappings
    pub invariants: Vec<TlaInvariant>,
    /// Evaluator mappings from the header
    pub evaluator_map: HashMap<String, Vec<String>>,
}

impl TlaSpec {
    /// Parse a TLA+ spec from content.
    pub fn parse(content: &str) -> Result<Self, ParseError> {
        let mut spec = TlaSpec {
            name: String::new(),
            content: content.to_string(),
            path: None,
            constants: Vec::new(),
            variables: Vec::new(),
            invariants: Vec::new(),
            evaluator_map: HashMap::new(),
        };

        // Parse module name
        for line in content.lines() {
            if line.contains("MODULE") {
                if let Some(name) = extract_module_name(line) {
                    spec.name = name;
                    break;
                }
            }
        }

        if spec.name.is_empty() {
            return Err(ParseError::NoModuleName);
        }

        // Parse evaluator mappings from header comments
        // Format: "* Line N: InvariantName -> evaluator1, evaluator2"
        for line in content.lines() {
            if line.contains("Line") && line.contains("->") {
                if let Some((inv_name, line_num, evaluators)) = parse_evaluator_mapping(line) {
                    spec.evaluator_map.insert(inv_name.clone(), evaluators.clone());
                    spec.invariants.push(TlaInvariant {
                        name: inv_name,
                        line: line_num,
                        description: None,
                        evaluators,
                    });
                }
            }
        }

        // Parse CONSTANTS
        let mut in_constants = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("CONSTANTS") {
                in_constants = true;
                // Handle inline constants
                if let Some(rest) = trimmed.strip_prefix("CONSTANTS") {
                    for constant in rest.split(',') {
                        let c = constant.trim().trim_end_matches(',');
                        if !c.is_empty() && !c.starts_with("\\*") {
                            spec.constants.push(c.to_string());
                        }
                    }
                }
                continue;
            }
            if in_constants {
                if trimmed.starts_with("VARIABLES") || trimmed.starts_with("----") {
                    in_constants = false;
                } else {
                    for constant in trimmed.split(',') {
                        let c = constant.trim().trim_end_matches(',');
                        if !c.is_empty() && !c.starts_with("\\*") {
                            spec.constants.push(c.to_string());
                        }
                    }
                }
            }
        }

        // Parse VARIABLES
        let mut in_variables = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("VARIABLES") {
                in_variables = true;
                // Handle inline variables
                if let Some(rest) = trimmed.strip_prefix("VARIABLES") {
                    for var in rest.split(',') {
                        let v = var.trim().trim_end_matches(',');
                        if !v.is_empty() && !v.starts_with("\\*") {
                            spec.variables.push(v.to_string());
                        }
                    }
                }
                continue;
            }
            if in_variables {
                if trimmed.starts_with("vars ==") || trimmed.starts_with("----") {
                    in_variables = false;
                } else if !trimmed.is_empty() && !trimmed.starts_with("\\*") {
                    for var in trimmed.split(',') {
                        let v = var.trim().trim_end_matches(',');
                        if !v.is_empty() && !v.starts_with("\\*") {
                            spec.variables.push(v.to_string());
                        }
                    }
                }
            }
        }

        // Parse invariant definitions with line numbers
        let lines: Vec<&str> = content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            // Look for invariant definitions: "InvariantName ==" at the start of line
            if trimmed.ends_with("==") && !trimmed.starts_with("\\*") && !trimmed.contains("THEN") {
                let name = trimmed.trim_end_matches("==").trim();
                if !name.is_empty()
                    && name.chars().next().unwrap().is_uppercase()
                    && !name.contains(' ')
                {
                    // Check if we already have this invariant from the header
                    let line_num = (i + 1) as u32;
                    if !spec.invariants.iter().any(|inv| inv.name == name) {
                        // Get description from preceding comment
                        let description = if i > 0 {
                            let prev = lines[i - 1].trim();
                            if prev.starts_with("\\*") || prev.starts_with("(*") {
                                Some(
                                    prev.trim_start_matches("\\*")
                                        .trim_start_matches("(*")
                                        .trim_end_matches("*)")
                                        .trim()
                                        .to_string(),
                                )
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        spec.invariants.push(TlaInvariant {
                            name: name.to_string(),
                            line: line_num,
                            description,
                            evaluators: Vec::new(),
                        });
                    }
                }
            }
        }

        Ok(spec)
    }

    /// Load a TLA+ spec from a file.
    pub fn from_file(path: &Path) -> Result<Self, ParseError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| ParseError::IoError(e.to_string()))?;

        let mut spec = Self::parse(&content)?;
        spec.path = Some(path.to_string_lossy().to_string());
        Ok(spec)
    }

    /// Get invariants that should be checked by a specific evaluator.
    pub fn invariants_for_evaluator(&self, evaluator: &str) -> Vec<&TlaInvariant> {
        self.invariants
            .iter()
            .filter(|inv| inv.evaluators.iter().any(|e| e == evaluator))
            .collect()
    }

    /// Format invariants for display.
    pub fn format_invariants(&self) -> String {
        self.invariants
            .iter()
            .map(|inv| {
                let evaluators = if inv.evaluators.is_empty() {
                    String::new()
                } else {
                    format!(" -> {}", inv.evaluators.join(", "))
                };
                format!("Line {}: {}{}", inv.line, inv.name, evaluators)
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Parse error types.
#[derive(Debug, Clone)]
pub enum ParseError {
    NoModuleName,
    IoError(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::NoModuleName => write!(f, "No MODULE declaration found"),
            ParseError::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for ParseError {}

/// Extract module name from MODULE line.
fn extract_module_name(line: &str) -> Option<String> {
    // Handle: "---- MODULE name ----" or "MODULE name"
    let stripped = line
        .trim()
        .trim_start_matches('-')
        .trim_end_matches('-')
        .trim();

    if let Some(rest) = stripped.strip_prefix("MODULE") {
        let name = rest.trim();
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    None
}

/// Parse evaluator mapping from comment.
/// Format: "* Line N: InvariantName -> evaluator1, evaluator2"
fn parse_evaluator_mapping(line: &str) -> Option<(String, u32, Vec<String>)> {
    let stripped = line
        .trim()
        .trim_start_matches('*')
        .trim_start_matches("\\*")
        .trim();

    // Find "Line N:"
    let after_line = stripped.strip_prefix("Line ")?.trim_start();
    let colon_pos = after_line.find(':')?;
    let line_num: u32 = after_line[..colon_pos].trim().parse().ok()?;

    let rest = &after_line[colon_pos + 1..];

    // Find "->"
    let arrow_pos = rest.find("->")?;
    let inv_name = rest[..arrow_pos].trim().to_string();
    let evaluators_str = rest[arrow_pos + 2..].trim();

    let evaluators: Vec<String> = evaluators_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Some((inv_name, line_num, evaluators))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SPEC: &str = r#"
---------------------------- MODULE test_stack ----------------------------
(*
 * Test Stack Specification
 *
 * EVALUATOR MAPPING
 * -----------------
 * Line 45: NoLostElements -> stateright, loom
 * Line 58: NoDuplicates -> stateright
 *)

CONSTANTS
    Elements,
    Threads

VARIABLES
    head,
    nodes,
    pushed

NoLostElements ==
    \A e \in pushed: e \in stack \/ e \in popped

NoDuplicates ==
    Len(stack) = Cardinality(Range(stack))

=============================================================================
"#;

    #[test]
    fn test_parse_module_name() {
        let spec = TlaSpec::parse(TEST_SPEC).unwrap();
        assert_eq!(spec.name, "test_stack");
    }

    #[test]
    fn test_parse_constants() {
        let spec = TlaSpec::parse(TEST_SPEC).unwrap();
        assert!(spec.constants.contains(&"Elements".to_string()));
        assert!(spec.constants.contains(&"Threads".to_string()));
    }

    #[test]
    fn test_parse_variables() {
        let spec = TlaSpec::parse(TEST_SPEC).unwrap();
        assert!(spec.variables.contains(&"head".to_string()));
        assert!(spec.variables.contains(&"nodes".to_string()));
        assert!(spec.variables.contains(&"pushed".to_string()));
    }

    #[test]
    fn test_parse_evaluator_mapping() {
        let spec = TlaSpec::parse(TEST_SPEC).unwrap();

        let no_lost = spec
            .invariants
            .iter()
            .find(|i| i.name == "NoLostElements")
            .unwrap();
        assert_eq!(no_lost.line, 45);
        assert!(no_lost.evaluators.contains(&"stateright".to_string()));
        assert!(no_lost.evaluators.contains(&"loom".to_string()));

        let no_dup = spec
            .invariants
            .iter()
            .find(|i| i.name == "NoDuplicates")
            .unwrap();
        assert_eq!(no_dup.line, 58);
        assert!(no_dup.evaluators.contains(&"stateright".to_string()));
    }

    #[test]
    fn test_invariants_for_evaluator() {
        let spec = TlaSpec::parse(TEST_SPEC).unwrap();

        let stateright_invs = spec.invariants_for_evaluator("stateright");
        assert_eq!(stateright_invs.len(), 2);

        let loom_invs = spec.invariants_for_evaluator("loom");
        assert_eq!(loom_invs.len(), 1);
    }

    #[test]
    fn test_format_invariants() {
        let spec = TlaSpec::parse(TEST_SPEC).unwrap();
        let formatted = spec.format_invariants();
        assert!(formatted.contains("NoLostElements"));
        assert!(formatted.contains("NoDuplicates"));
    }
}
