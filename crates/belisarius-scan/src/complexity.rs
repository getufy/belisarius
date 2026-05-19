//! Cyclomatic + cognitive complexity for a function body.
//!
//! `cc_branch_kinds` vs `cog_branch_kinds` is split on purpose: a `match`/`switch`
//! statement contributes +1 to cognitive once (per SonarSource 2016) but each
//! `match_arm`/`switch_case` contributes +1 to cyclomatic (each arm is a path).
//! `fn_kinds` lets us stop descending into nested function bodies — they're
//! counted on their own when the outer pass reaches them.

use tree_sitter::Node;

#[derive(Debug, Clone, Copy)]
pub struct Complexity {
    pub cyclomatic: u32,
    pub cognitive: u32,
}

pub struct LangSpec {
    /// Each occurrence adds +1 to cyclomatic.
    pub cc_branch_kinds: &'static [&'static str],
    /// Each occurrence adds (1 + nesting) to cognitive.
    pub cog_branch_kinds: &'static [&'static str],
    /// AST kinds where a short-circuit boolean operator might appear.
    pub logic_kinds: &'static [&'static str],
    /// Operator strings counted as logical short-circuits. Empty if `logic_kinds`
    /// already implies the operator (e.g. Python `boolean_operator`).
    pub logic_operators: &'static [&'static str],
    /// Entering these kinds bumps the cognitive nesting depth by 1.
    pub nest_kinds: &'static [&'static str],
    /// Function-definition kinds. We stop descending into these so a nested
    /// function's branches don't inflate the outer function's metrics.
    pub fn_kinds: &'static [&'static str],
}

pub const RUST: LangSpec = LangSpec {
    cc_branch_kinds: &[
        "if_expression",
        "while_expression",
        "loop_expression",
        "for_expression",
        "match_arm",
        "try_expression",
    ],
    cog_branch_kinds: &[
        "if_expression",
        "while_expression",
        "loop_expression",
        "for_expression",
        "match_expression",
        "try_expression",
    ],
    logic_kinds: &["binary_expression"],
    logic_operators: &["&&", "||"],
    nest_kinds: &[
        "if_expression",
        "while_expression",
        "loop_expression",
        "for_expression",
        "match_expression",
    ],
    fn_kinds: &["function_item", "closure_expression"],
};

pub const TS_JS: LangSpec = LangSpec {
    cc_branch_kinds: &[
        "if_statement",
        "while_statement",
        "do_statement",
        "for_statement",
        "for_in_statement",
        "for_of_statement",
        "ternary_expression",
        "switch_case",
        "switch_default",
        "catch_clause",
    ],
    cog_branch_kinds: &[
        "if_statement",
        "while_statement",
        "do_statement",
        "for_statement",
        "for_in_statement",
        "for_of_statement",
        "ternary_expression",
        "switch_statement",
        "catch_clause",
    ],
    logic_kinds: &["binary_expression"],
    logic_operators: &["&&", "||", "??"],
    nest_kinds: &[
        "if_statement",
        "while_statement",
        "do_statement",
        "for_statement",
        "for_in_statement",
        "for_of_statement",
        "switch_statement",
        "try_statement",
    ],
    fn_kinds: &[
        "function_declaration",
        "method_definition",
        "function_signature",
        "function",
        "function_expression",
        "arrow_function",
        "generator_function_declaration",
        "generator_function",
    ],
};

pub const PYTHON: LangSpec = LangSpec {
    cc_branch_kinds: &[
        "if_statement",
        "elif_clause",
        "while_statement",
        "for_statement",
        "except_clause",
        "conditional_expression",
    ],
    cog_branch_kinds: &[
        "if_statement",
        "elif_clause",
        "while_statement",
        "for_statement",
        "except_clause",
        "conditional_expression",
    ],
    logic_kinds: &["boolean_operator"],
    logic_operators: &[],
    nest_kinds: &[
        "if_statement",
        "while_statement",
        "for_statement",
        "try_statement",
    ],
    fn_kinds: &["function_definition", "lambda"],
};

pub const GO: LangSpec = LangSpec {
    cc_branch_kinds: &[
        "if_statement",
        "for_statement",
        "expression_case",
        "type_case",
        "communication_case",
    ],
    cog_branch_kinds: &[
        "if_statement",
        "for_statement",
        "expression_switch_statement",
        "type_switch_statement",
        "select_statement",
    ],
    logic_kinds: &["binary_expression"],
    logic_operators: &["&&", "||"],
    nest_kinds: &[
        "if_statement",
        "for_statement",
        "expression_switch_statement",
        "type_switch_statement",
        "select_statement",
    ],
    fn_kinds: &["function_declaration", "method_declaration", "func_literal"],
};

pub fn compute(spec: &LangSpec, body: Node, source: &[u8]) -> Complexity {
    let mut cc: u32 = 1; // base: one path through the function
    let mut cog: u32 = 0;
    walk(spec, body, source, 0, &mut cc, &mut cog, true);
    Complexity {
        cyclomatic: cc,
        cognitive: cog,
    }
}

fn walk(
    spec: &LangSpec,
    node: Node,
    source: &[u8],
    nesting: u32,
    cc: &mut u32,
    cog: &mut u32,
    is_root: bool,
) {
    let kind = node.kind();

    // Bail out on nested function bodies — they're walked on their own pass.
    if !is_root && spec.fn_kinds.contains(&kind) {
        return;
    }

    if spec.cc_branch_kinds.contains(&kind) {
        *cc += 1;
    }
    if spec.cog_branch_kinds.contains(&kind) {
        *cog += 1 + nesting;
    }

    if spec.logic_kinds.contains(&kind) {
        if spec.logic_operators.is_empty() {
            *cc += 1;
            *cog += 1;
        } else if let Some(op) = node.child_by_field_name("operator") {
            let text = op.utf8_text(source).unwrap_or("");
            if spec.logic_operators.contains(&text) {
                *cc += 1;
                *cog += 1;
            }
        } else {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                let text = child.utf8_text(source).unwrap_or("");
                if spec.logic_operators.contains(&text) {
                    *cc += 1;
                    *cog += 1;
                    break;
                }
            }
        }
    }

    let nest_inc = if spec.nest_kinds.contains(&kind) {
        1
    } else {
        0
    };
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(spec, child, source, nesting + nest_inc, cc, cog, false);
    }
}
