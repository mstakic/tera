/// Tests for `Tera::set_loosely_render` — the lenient rendering mode where
/// variables missing from the context are rendered using the template-defined
/// `undefined_var_fallback` variable (set via
/// `{% set undefined_var_fallback = '...' %}`), or as
/// `[ERROR Rendering segment: Variable \`NAME\` not found]` when that variable
/// is absent.
///
/// The tests are grouped into:
///   A. Basic VariableBlock output — what works
///   B. Filters on missing variables
///   C. Conditionals (if / not / and / or)
///   D. Testers (is defined / is undefined / is string / …)
///   E. Things that STILL fail despite loose rendering being enabled
///   F. Subtle / surprising rendering behaviours
///   G. Configuration lifecycle
use serde_json::json;
// Import std::error::Error as a trait to make `.source()` available.
use std::error::Error as StdError;

use crate::context::Context;
use crate::errors::ErrorKind;
use crate::tera::Tera;

// ── helpers ──────────────────────────────────────────────────────────────────

/// Build a Tera instance with loose rendering enabled.
fn tera_loosely() -> Tera {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    tera
}

/// Render `template` in loose mode with `fallback` as the
/// `undefined_var_fallback` template variable.
///
/// The fallback is injected by prepending
/// `{% set undefined_var_fallback = '...' %}` to the template.
fn render(template: &str, ctx: &Context, fallback: &str) -> String {
    let mut tera = tera_loosely();
    let full = format!("{{% set undefined_var_fallback = '{}' %}}{}", fallback, template);
    tera.add_raw_template("tpl", &full).unwrap();
    tera.render("tpl", ctx).unwrap()
}

fn render_err(template: &str, ctx: &Context, fallback: &str) -> crate::errors::Error {
    let mut tera = tera_loosely();
    let full = format!("{{% set undefined_var_fallback = '{}' %}}{}", fallback, template);
    tera.add_raw_template("tpl", &full).unwrap();
    tera.render("tpl", ctx).unwrap_err()
}

// ── A. Basic VariableBlock output ─────────────────────────────────────────────

#[test]
fn test_simple_missing_variable() {
    assert_eq!(render("{{ missing }}", &Context::new(), "N/A"), "N/A");
}

#[test]
fn test_multiple_missing_variables_each_replaced() {
    // Every missing variable is independently replaced.
    assert_eq!(render("{{ a }}{{ b }}{{ c }}", &Context::new(), "N/A"), "N/AN/AN/A");
}

#[test]
fn test_defined_variable_unaffected() {
    // Defined variables still render normally alongside missing ones.
    let mut ctx = Context::new();
    ctx.insert("present", &"hello");
    assert_eq!(render("{{ present }}-{{ missing }}", &ctx, "N/A"), "hello-N/A");
}

#[test]
fn test_missing_dotted_path_missing_root() {
    // Fully missing root renders as fallback.
    assert_eq!(render("{{ missing.attr }}", &Context::new(), "N/A"), "N/A");
}

#[test]
fn test_missing_dotted_path_existing_root_missing_attr() {
    // Root object exists but the requested attribute does not.
    let mut ctx = Context::new();
    ctx.insert("obj", &json!({"existing": "val"}));
    assert_eq!(render("{{ obj.missing_key }}", &ctx, "N/A"), "N/A");
}

#[test]
fn test_missing_literal_array_index() {
    // Literal numeric index on a missing variable renders as fallback.
    assert_eq!(render("{{ missing[0] }}", &Context::new(), "N/A"), "N/A");
}

#[test]
fn test_custom_fallback_value() {
    // The fallback string is fully configurable.
    assert_eq!(render("{{ missing }}", &Context::new(), "UNKNOWN"), "UNKNOWN");
}

#[test]
fn test_empty_string_fallback() {
    // An empty string is a valid fallback.
    assert_eq!(render("{{ missing }}", &Context::new(), ""), "");
}

// ── B. Filters on missing variables ──────────────────────────────────────────

#[test]
fn test_filter_is_not_applied_to_fallback() {
    // The filter pipeline is bypassed when the variable is missing.
    // The raw fallback value is written, NOT the filtered version.
    // Here "na" is NOT uppercased to "NA".
    assert_eq!(render("{{ missing | upper }}", &Context::new(), "na"), "na");
}

#[test]
fn test_explicit_default_filter_takes_priority_over_fallback() {
    // An explicit `| default(value=...)` filter is evaluated before the
    // loose-render fallback kicks in, so it wins.
    let result = render(r#"{{ missing | default(value="explicit") }}"#, &Context::new(), "N/A");
    assert_eq!(result, "explicit");
}

#[test]
fn test_default_filter_without_value_arg_fails() {
    // `| default` with no `value` argument is itself a template error;
    // it fails regardless of undefined_variable_value.
    let err = render_err("{{ missing | default }}", &Context::new(), "N/A");
    assert!(
        err.to_string().contains("Failed to render"),
        "unexpected error: {err}"
    );
}

// ── C. Conditionals ───────────────────────────────────────────────────────────

#[test]
fn test_if_missing_is_falsy() {
    // A missing variable in an `{% if %}` condition is treated as false,
    // so the block is NOT rendered.
    let result = render("{% if missing %}yes{% else %}no{% endif %}", &Context::new(), "N/A");
    assert_eq!(result, "no");
}

#[test]
fn test_if_not_missing_is_truthy() {
    // `not missing` follows the pre-existing special case:
    // negating an undefined identifier is truthy, so the block IS rendered.
    let result = render(
        "{% if not missing %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "yes");
}

#[test]
fn test_and_with_missing_lhs_is_false_no_error() {
    // `missing and defined_true` short-circuits to false without an error,
    // because the Ident branch in eval_as_bool uses unwrap_or(false).
    let mut ctx = Context::new();
    ctx.insert("present", &true);
    let result =
        render("{% if missing and present %}yes{% else %}no{% endif %}", &ctx, "N/A");
    assert_eq!(result, "no");
}

#[test]
fn test_and_with_missing_rhs_is_false_no_error() {
    let mut ctx = Context::new();
    ctx.insert("present", &true);
    let result =
        render("{% if present and missing %}yes{% else %}no{% endif %}", &ctx, "N/A");
    assert_eq!(result, "no");
}

#[test]
fn test_or_with_missing_lhs_uses_rhs() {
    // `missing or present` falls back to the rhs operand.
    let mut ctx = Context::new();
    ctx.insert("present", &true);
    let result =
        render("{% if missing or present %}yes{% else %}no{% endif %}", &ctx, "N/A");
    assert_eq!(result, "yes");
}

#[test]
fn test_or_with_both_missing_is_false() {
    let result = render(
        "{% if missing1 or missing2 %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

// ── D. Testers ────────────────────────────────────────────────────────────────

#[test]
fn test_is_defined_returns_false_for_missing() {
    // `missing is defined` must still return false; lookup returns an error
    // and the tester sees None.
    let result = render(
        "{% if missing is defined %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

#[test]
fn test_is_not_defined_returns_true_for_missing() {
    let result = render(
        "{% if missing is not defined %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "yes");
}

#[test]
fn test_is_undefined_returns_true_for_missing() {
    let result = render(
        "{% if missing is undefined %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "yes");
}

#[test]
fn test_is_string_on_missing_fails() {
    // Unlike `defined`/`undefined`, the `string` tester requires a value to be
    // present and errors with "called on an undefined variable" when it is not.
    let err = render_err(
        "{% if missing is string %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert!(
        err.to_string().contains("Failed to render"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_is_number_on_missing_fails() {
    // Same as `is string` — the `number` tester errors on an undefined variable.
    let err = render_err(
        "{% if missing is number %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert!(
        err.to_string().contains("Failed to render"),
        "unexpected error: {err}"
    );
}

#[test]
fn test_is_iterable_on_missing_fails() {
    // Same — the `iterable` tester errors on an undefined variable.
    let err = render_err(
        "{% if missing is iterable %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert!(
        err.to_string().contains("Failed to render"),
        "unexpected error: {err}"
    );
}

// ── E. Things that STILL fail despite the fallback ────────────────────────────

/// Math operations: the VariableNotFound error is wrapped inside a generic
/// `Msg` error by the math evaluator, so the VariableBlock fallback handler
/// never sees a `VariableNotFound` kind and the render fails.
#[test]
fn test_math_with_missing_variable_fails() {
    let err = render_err("{{ missing + 1 }}", &Context::new(), "N/A");
    assert!(err.source().is_some(), "expected a chained error, got: {}", err);
}

/// Variable used as an array index: the sub-variable lookup error is wrapped
/// as a `Msg` by `evaluate_sub_variables`, so the fallback does not apply.
#[test]
fn test_variable_array_index_missing_fails() {
    let err = render_err("{{ arr[missing_idx] }}", &Context::new(), "N/A");
    assert!(err.source().is_some(), "expected a chained error, got: {}", err);
}

/// Numeric comparisons (>, >=, <, <=) use eval_expr_as_number which calls
/// lookup_ident directly; the VariableNotFound propagates.
#[test]
fn test_numeric_comparison_with_missing_fails() {
    let err = render_err("{% if missing > 5 %}yes{% endif %}", &Context::new(), "N/A");
    assert!(err.source().is_some(), "expected a chained error, got: {}", err);
}

/// For loops: safe_eval_expression propagates the error because the
/// for-loop container is not a VariableBlock context.
#[test]
fn test_for_loop_over_missing_variable_fails() {
    let err = render_err(
        "{% for item in missing %}{{ item }}{% endfor %}",
        &Context::new(),
        "N/A",
    );
    assert!(err.source().is_some(), "expected a chained error, got: {}", err);
}

// ── H. Newly supported: equality, set, and `in` with undefined vars ───────────

/// `{% if missing == "x" %}` resolves to false — neither side errors, the
/// comparison is simply treated as unequal.
#[test]
fn test_eq_with_missing_lhs_is_false() {
    let result = render(
        r#"{% if missing == "x" %}yes{% else %}no{% endif %}"#,
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

/// `{% if missing != "x" %}` also resolves to false (the pair is incomparable).
#[test]
fn test_neq_with_missing_lhs_is_false() {
    let result = render(
        r#"{% if missing != "x" %}yes{% else %}no{% endif %}"#,
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

/// `{% if "x" == missing %}` — undefined on the RHS behaves the same way.
#[test]
fn test_eq_with_missing_rhs_is_false() {
    let result = render(
        r#"{% if "x" == missing %}yes{% else %}no{% endif %}"#,
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

/// Outer negation still applies: `not (missing == "x")` is true because the
/// inner comparison is false and `not false` = true.
#[test]
fn test_not_eq_with_missing_is_true() {
    let result = render(
        r#"{% if not missing == "x" %}yes{% else %}no{% endif %}"#,
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "yes");
}

/// Two defined values compared normally — the fallback path is not taken.
#[test]
fn test_eq_with_both_defined_still_works() {
    let mut ctx = Context::new();
    ctx.insert("a", &"hello");
    ctx.insert("b", &"hello");
    let result = render(
        "{% if a == b %}yes{% else %}no{% endif %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "yes");
}

/// `{% set x = missing %}{{ x }}` — the set tag assigns the fallback string,
/// so rendering `x` outputs the fallback.
#[test]
fn test_set_missing_variable_assigns_fallback() {
    let result = render("{% set x = missing %}{{ x }}", &Context::new(), "N/A");
    assert_eq!(result, "N/A");
}

/// After `{% set x = missing %}`, `x` holds the fallback string (a non-empty
/// string), which is truthy. A conditional on `x` will enter the block.
#[test]
fn test_set_missing_then_if_x_is_truthy() {
    let result = render(
        "{% set x = missing %}{% if x %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    // x = "N/A" (non-empty string) → truthy
    assert_eq!(result, "yes");
}

/// `{% set x = missing %}` with an empty-string fallback — `x` is an empty
/// string, which is falsy.
#[test]
fn test_set_missing_with_empty_fallback_is_falsy() {
    let result = render(
        "{% set x = missing %}{% if x %}yes{% else %}no{% endif %}",
        &Context::new(),
        "",
    );
    // x = "" → falsy
    assert_eq!(result, "no");
}

/// `{% if missing in ["a", "b"] %}` resolves to false — undefined LHS is
/// treated as "not present".
#[test]
fn test_in_with_missing_lhs_is_false() {
    let result = render(
        r#"{% if missing in ["a", "b"] %}yes{% else %}no{% endif %}"#,
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

/// `{% if missing not in ["a", "b"] %}` resolves to true — undefined is not
/// in the list, so `not in` is true.
#[test]
fn test_not_in_with_missing_lhs_is_true() {
    let result = render(
        r#"{% if missing not in ["a", "b"] %}yes{% else %}no{% endif %}"#,
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "yes");
}

/// `{% if "a" in missing %}` — undefined on the RHS also resolves to false.
#[test]
fn test_in_with_missing_rhs_is_false() {
    let result = render(
        r#"{% if "a" in missing %}yes{% else %}no{% endif %}"#,
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

// ── F. Subtle / surprising behaviours ────────────────────────────────────────

/// String concatenation: the *entire* `~` expression renders as the fallback,
/// not just the missing part. The StringConcat evaluator short-circuits on the
/// first missing identifier.
#[test]
fn test_string_concat_with_missing_var_is_whole_fallback() {
    // "prefix" ~ missing → "N/A", NOT "prefixN/A"
    let result = render(r#"{{ "prefix" ~ missing }}"#, &Context::new(), "N/A");
    assert_eq!(result, "N/A");
}

#[test]
fn test_string_concat_missing_then_present_is_fallback() {
    // missing ~ present → "N/A" (fails on the first ident)
    let mut ctx = Context::new();
    ctx.insert("present", &" world");
    let result = render(r#"{{ missing ~ present }}"#, &ctx, "N/A");
    assert_eq!(result, "N/A");
}

/// Autoescape: the fallback value is written with a raw `write!` call and
/// does NOT pass through the HTML-escape function, even when autoescape is
/// enabled for the template. Callers should ensure the fallback is safe.
#[test]
fn test_fallback_not_html_escaped_in_autoescape_context() {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    // The ".html" suffix enables autoescaping.
    // The undefined_var_fallback contains raw HTML — it will be written as-is
    // because loose_fallback_value bypasses the escape function.
    tera.add_raw_template(
        "tpl.html",
        "{% set undefined_var_fallback = '<b>N/A</b>' %}{{ missing }}",
    )
    .unwrap();

    let result = tera.render("tpl.html", &Context::new()).unwrap();
    // Raw HTML is output as-is; no escaping applied to the fallback.
    assert_eq!(result, "<b>N/A</b>");
}

/// Defined variables ARE still escaped in autoescape mode; only the fallback
/// bypasses the escape function.
#[test]
fn test_defined_variable_still_escaped_in_autoescape_context() {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    tera.add_raw_template(
        "tpl.html",
        "{% set undefined_var_fallback = 'N/A' %}{{ xss }}{{ missing }}",
    )
    .unwrap();

    let mut ctx = Context::new();
    ctx.insert("xss", &"<script>");

    let result = tera.render("tpl.html", &ctx).unwrap();
    assert_eq!(result, "&lt;script&gt;N/A");
}

// ── G. Configuration lifecycle ────────────────────────────────────────────────

/// Toggling `set_loosely_render` on and off controls whether missing variables
/// abort rendering.
///
/// - loose on, no `undefined_var_fallback` → renders `[ERROR Rendering segment: Variable \`...\` not found]`
/// - loose on, `undefined_var_fallback = 'N/A'` → renders "N/A"
/// - loose off (default) → hard render error
#[test]
fn test_loose_render_toggle_controls_strict_mode() {
    let mut tera = Tera::default();
    tera.add_raw_template("tpl", "{{ missing }}").unwrap();
    tera.add_raw_template(
        "tpl_with_fallback",
        "{% set undefined_var_fallback = 'N/A' %}{{ missing }}",
    )
    .unwrap();

    // Strict mode (default) — must error.
    assert!(tera.render("tpl", &Context::new()).is_err());

    // Loose mode without template fallback — renders error segment tag.
    tera.set_loosely_render(true);
    let result = tera.render("tpl", &Context::new()).unwrap();
    assert!(
        result.starts_with("[ERROR Rendering segment:"),
        "unexpected output: {result}"
    );

    // Loose mode with template fallback — renders the fallback value.
    assert_eq!(tera.render("tpl_with_fallback", &Context::new()).unwrap(), "N/A");

    // Disable loose rendering — back to strict mode.
    tera.set_loosely_render(false);
    assert!(tera.render("tpl", &Context::new()).is_err());
}

/// With no fallback set (default), missing variables produce an error whose
/// source contains the "not found in context" message.
#[test]
fn test_without_fallback_missing_variable_is_error() {
    let mut tera = Tera::default();
    tera.add_raw_template("tpl", "{{ missing }}").unwrap();
    let err = tera.render("tpl", &Context::new()).unwrap_err();
    assert!(err.source().is_some());
    assert!(err.source().unwrap().to_string().contains("not found in context"));
}

/// The error source for a missing variable is a VariableNotFound kind.
#[test]
fn test_without_fallback_error_kind_is_variable_not_found() {
    let mut tera = Tera::default();
    tera.add_raw_template("tpl", "{{ missing }}").unwrap();
    let err = tera.render("tpl", &Context::new()).unwrap_err();
    let source = err.source().unwrap();
    // Downcast to tera::Error to inspect the kind.
    let tera_err = source.downcast_ref::<crate::errors::Error>().unwrap();
    assert!(
        matches!(tera_err.kind, ErrorKind::VariableNotFound(_)),
        "expected VariableNotFound, got {:?}",
        tera_err.kind
    );
}

/// The fallback value is per-template: two templates can define different
/// `undefined_var_fallback` values in the same Tera instance.
#[test]
fn test_fallback_is_per_template() {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    tera.add_raw_template(
        "tpl_na",
        "{% set undefined_var_fallback = 'N/A' %}{{ missing }}",
    )
    .unwrap();
    tera.add_raw_template(
        "tpl_unk",
        "{% set undefined_var_fallback = 'UNKNOWN' %}{{ missing }}",
    )
    .unwrap();

    assert_eq!(tera.render("tpl_na", &Context::new()).unwrap(), "N/A");
    assert_eq!(tera.render("tpl_unk", &Context::new()).unwrap(), "UNKNOWN");
}

// ── I. Complex / deep templates ───────────────────────────────────────────────

// --- Nesting ---

/// Missing var at the inner level of nested ifs: the outer condition is true,
/// but the inner condition uses an undefined variable — it evaluates to false
/// and falls through to the else branch.
#[test]
fn test_nested_if_missing_at_inner_level() {
    let mut ctx = Context::new();
    ctx.insert("outer", &true);
    let result = render(
        "{% if outer %}{% if missing %}inner{% else %}fallthrough{% endif %}{% endif %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "fallthrough");
}

/// Three levels of nesting; the undefined variable sits at the middle level,
/// preventing the innermost block from ever being reached.
#[test]
fn test_three_level_nested_if_missing_at_middle() {
    let mut ctx = Context::new();
    ctx.insert("l1", &true);
    ctx.insert("l3", &true);
    let result = render(
        "{% if l1 %}A{% if l2 %}B{% if l3 %}C{% endif %}{% endif %}{% endif %}",
        &ctx,
        "N/A",
    );
    // l2 is undefined → false → B and C are never rendered
    assert_eq!(result, "A");
}

/// Undefined variable inside an else branch still renders the fallback.
#[test]
fn test_missing_in_else_branch() {
    let mut ctx = Context::new();
    ctx.insert("cond", &false);
    let result = render(
        "{% if cond %}yes{% else %}{{ missing }}{% endif %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "N/A");
}

/// if-elif-else: the first branch tests a missing variable (false), the elif
/// matches a defined value — rendering continues from elif.
#[test]
fn test_if_elif_else_missing_first_branch() {
    let mut ctx = Context::new();
    ctx.insert("status", &"pending");
    let result = render(
        r#"{% if missing == "active" %}active{% elif status == "pending" %}pending{% else %}other{% endif %}"#,
        &ctx,
        "N/A",
    );
    assert_eq!(result, "pending");
}

// --- Dotted paths ---

/// Intermediate key in a dotted path is missing; the whole expression becomes
/// the fallback.
#[test]
fn test_deep_dotted_path_intermediate_missing() {
    let mut ctx = Context::new();
    ctx.insert("obj", &json!({"level1": {"level2": "value"}}));
    // obj and obj.level1 exist, but obj.level1.nope does not
    assert_eq!(render("{{ obj.level1.nope }}", &ctx, "N/A"), "N/A");
    // obj exists, obj.nope does not — further keys don't matter
    assert_eq!(render("{{ obj.nope.level2 }}", &ctx, "N/A"), "N/A");
}

/// Undefined dotted path used inside a condition.
#[test]
fn test_deep_dotted_path_in_condition() {
    let mut ctx = Context::new();
    ctx.insert("config", &json!({"enabled": true}));
    // config.timeout is missing → condition is false
    let result = render(
        "{% if config.timeout %}slow{% else %}default{% endif %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "default");
}

/// Undefined dotted path in an equality comparison.
#[test]
fn test_obj_missing_attr_eq_comparison() {
    let mut ctx = Context::new();
    ctx.insert("user", &json!({"name": "Alice"}));
    // user.role is missing → eq returns false
    let result = render(
        r#"{% if user.role == "admin" %}admin{% else %}guest{% endif %}"#,
        &ctx,
        "N/A",
    );
    assert_eq!(result, "guest");
}

/// Undefined dotted path as the LHS of `in`.
#[test]
fn test_obj_missing_attr_in_operator() {
    let mut ctx = Context::new();
    ctx.insert("user", &json!({"name": "Alice"}));
    let result = render(
        r#"{% if user.role in ["admin", "moderator"] %}privileged{% else %}regular{% endif %}"#,
        &ctx,
        "N/A",
    );
    assert_eq!(result, "regular");
}

// --- Multiple / repeated missing vars ---

/// Every occurrence of a missing variable in the same template gets the
/// fallback independently.
#[test]
fn test_repeated_reference_to_same_missing_var() {
    let result = render(
        "{{ x }}, {{ x }}, {{ x }}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "N/A, N/A, N/A");
}

/// Mixture of defined and undefined fields across a multi-field template.
#[test]
fn test_multiple_missing_fields_with_defined_ones() {
    let mut ctx = Context::new();
    ctx.insert("username", &"alice");
    let result = render(
        "User: {{ username }}, Email: {{ email }}, Score: {{ score }}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "User: alice, Email: N/A, Score: N/A");
}

/// Both the output block and the condition reference the same missing variable;
/// the output renders the fallback while the condition evaluates to false.
#[test]
fn test_output_and_condition_same_missing_var() {
    let result = render(
        "val={{ missing }}, set={% if missing %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "val=N/A, set=no");
}

// --- Boolean logic ---

/// Or-chain where every operand is undefined: all evaluate to false.
#[test]
fn test_or_chain_all_missing() {
    let result = render(
        "{% if a or b or c %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    assert_eq!(result, "no");
}

/// And-chain with a missing variable in the middle: the whole condition
/// short-circuits to false.
#[test]
fn test_and_chain_missing_in_middle() {
    let mut ctx = Context::new();
    ctx.insert("a", &true);
    ctx.insert("c", &true);
    let result = render(
        "{% if a and missing and c %}yes{% else %}no{% endif %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "no");
}

/// Missing on the LHS of an equality inside an or-chain: the undefined
/// comparison is false but the other operand (defined, true) wins.
#[test]
fn test_eq_with_missing_in_or_chain_defined_side_wins() {
    let mut ctx = Context::new();
    ctx.insert("status", &"active");
    let result = render(
        r#"{% if missing == "x" or status == "active" %}yes{% else %}no{% endif %}"#,
        &ctx,
        "N/A",
    );
    assert_eq!(result, "yes");
}

/// A complex boolean `(a == x or b == y) and flag` where both comparisons
/// involve undefined variables — the whole condition is false.
#[test]
fn test_complex_boolean_all_comparisons_missing() {
    let mut ctx = Context::new();
    ctx.insert("flag", &true);
    let result = render(
        r#"{% if (a == "x" or b == "y") and flag %}yes{% else %}no{% endif %}"#,
        &ctx,
        "N/A",
    );
    // (false or false) and true → false
    assert_eq!(result, "no");
}

/// A variable compared with itself when both sides are undefined evaluates to
/// false — the two lookups each produce VariableNotFound independently.
#[test]
fn test_missing_var_compared_with_itself_is_false() {
    let result = render(
        "{% if missing == missing %}yes{% else %}no{% endif %}",
        &Context::new(),
        "N/A",
    );
    // Not "equal to itself": both sides are undefined, comparison → false
    assert_eq!(result, "no");
}

// --- `in` with defined collections ---

/// Undefined LHS checked against a defined array: treated as "not present".
#[test]
fn test_in_missing_lhs_defined_array() {
    let mut ctx = Context::new();
    ctx.insert("allowed", &vec!["admin", "user", "moderator"]);
    let result = render(
        "{% if user_role in allowed %}ok{% else %}denied{% endif %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "denied");
}

/// `not in` with undefined LHS and defined array: undefined is not in the
/// list, so `not in` is true.
#[test]
fn test_not_in_missing_lhs_defined_array() {
    let mut ctx = Context::new();
    ctx.insert("blocked", &vec!["banned1", "banned2"]);
    let result = render(
        "{% if user_role not in blocked %}allowed{% else %}blocked{% endif %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "allowed");
}

// --- set interactions ---

/// `{% set x = missing | upper %}` — the filter pipeline on a missing variable
/// is never executed; the set tag assigns the fallback string directly.
#[test]
fn test_set_from_missing_with_filter_assigns_fallback() {
    let result = render(
        "{% set x = missing | upper %}{{ x }}",
        &Context::new(),
        "N/A",
    );
    // filter is skipped; x = "N/A" (not "N/A" uppercased)
    assert_eq!(result, "N/A");
}

// ── J. For-loop contexts ───────────────────────────────────────────────────────

/// A global missing variable referenced inside a for loop body is replaced
/// with the fallback on every iteration.
#[test]
fn test_global_missing_inside_for_loop() {
    let mut ctx = Context::new();
    ctx.insert("items", &vec!["a", "b", "c"]);
    let result = render(
        "{% for item in items %}{{ item }}/{{ sep }} {% endfor %}",
        &ctx,
        "N/A",
    );
    // `sep` is missing → fallback on every iteration
    assert_eq!(result, "a/N/A b/N/A c/N/A ");
}

/// Missing attribute on each loop object: the attribute is absent on all items,
/// so every iteration outputs the fallback.
#[test]
fn test_missing_attr_on_every_loop_item() {
    let mut ctx = Context::new();
    ctx.insert("users", &json!([{"name": "Alice"}, {"name": "Bob"}]));
    let result = render(
        "{% for u in users %}{{ u.name }}:{{ u.email }} {% endfor %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "Alice:N/A Bob:N/A ");
}

/// `{% if item.attr %}` used as a guard: when the attribute is missing the
/// condition is false and the block body is never rendered.
#[test]
fn test_if_guard_on_missing_attr_skips_body() {
    let mut ctx = Context::new();
    ctx.insert("users", &json!([{"name": "Alice"}, {"name": "Bob"}]));
    let result = render(
        "{% for u in users %}{% if u.active %}{{ u.name }}{% endif %}{% endfor %}",
        &ctx,
        "N/A",
    );
    // active is missing on both items → condition false → nothing rendered
    assert_eq!(result, "");
}

/// Loop items where the attribute is present on some but absent on others:
/// the absent ones render the fallback, present ones render normally.
#[test]
fn test_missing_attr_on_some_loop_items() {
    let mut ctx = Context::new();
    ctx.insert("users", &json!([
        {"name": "Alice"},
        {"name": "Bob", "email": "bob@example.com"},
        {"name": "Carol"}
    ]));
    let result = render(
        "{% for u in users %}{{ u.name }}:{{ u.email }} {% endfor %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "Alice:N/A Bob:bob@example.com Carol:N/A ");
}

/// A conditional display pattern inside a loop: show an optional badge only
/// when the attribute is present; missing → condition false → badge skipped.
#[test]
fn test_conditional_badge_on_missing_attr_in_loop() {
    let mut ctx = Context::new();
    ctx.insert("products", &json!([
        {"name": "Apple",  "price": 1},
        {"name": "Banana", "price": 2, "discount": 10}
    ]));
    let result = render(
        "{% for p in products %}{{ p.name }}/${{ p.price }}{% if p.discount %}(-{{ p.discount }}%){% endif %} {% endfor %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "Apple/$1 Banana/$2(-10%) ");
}

/// Equality check on a missing attribute inside a loop: the undefined side
/// makes the comparison false, falling through to the else branch.
#[test]
fn test_eq_on_missing_attr_inside_loop() {
    let mut ctx = Context::new();
    ctx.insert("users", &json!([
        {"name": "Alice"},
        {"name": "Bob", "role": "admin"}
    ]));
    let result = render(
        r#"{% for u in users %}{{ u.name }}={% if u.role == "admin" %}admin{% else %}user{% endif %} {% endfor %}"#,
        &ctx,
        "N/A",
    );
    assert_eq!(result, "Alice=user Bob=admin ");
}

/// `in` check on a missing attribute inside a loop: absent attribute is
/// treated as "not present", present attribute is checked normally.
#[test]
fn test_in_on_missing_attr_inside_loop() {
    let mut ctx = Context::new();
    ctx.insert("users", &json!([
        {"name": "Alice"},
        {"name": "Bob",   "role": "mod"},
        {"name": "Carol", "role": "admin"}
    ]));
    let result = render(
        r#"{% for u in users %}{% if u.role in ["admin", "mod"] %}+{% else %}-{% endif %}{% endfor %}"#,
        &ctx,
        "N/A",
    );
    assert_eq!(result, "-++");
}

/// `{% set %}` inside a loop from a missing attribute: the variable is
/// assigned the fallback for items that lack the attribute.
#[test]
fn test_set_from_missing_attr_inside_loop() {
    let mut ctx = Context::new();
    ctx.insert("items", &json!([
        {"value": 1},
        {"value": 2, "label": "two"},
        {"value": 3}
    ]));
    let result = render(
        "{% for item in items %}{% set lbl = item.label %}{{ lbl }} {% endfor %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "N/A two N/A ");
}

/// Compound case: a global missing variable and a per-item missing attribute
/// interact inside the same loop. The condition on the per-item attribute
/// falls through to an else that uses the global missing variable.
#[test]
fn test_compound_global_missing_and_attr_missing_in_loop() {
    let mut ctx = Context::new();
    ctx.insert("records", &json!([
        {"id": 1, "value": 10},
        {"id": 2},
        {"id": 3, "value": 30}
    ]));
    let result = render(
        "{% for r in records %}{{ r.id }}:{% if r.value %}{{ r.value }}{% else %}{{ default_val }}{% endif %} {% endfor %}",
        &ctx,
        "N/A",
    );
    // record 2: r.value missing → condition false → else → {{ default_val }} (global missing) → "N/A"
    assert_eq!(result, "1:10 2:N/A 3:30 ");
}

/// `{% if missing %}` output block vs a defined variable output in the same
/// loop: the condition gate works per-item independently of {{ }} output.
#[test]
fn test_condition_gate_and_output_per_item_in_loop() {
    let mut ctx = Context::new();
    ctx.insert("items", &json!([
        {"id": 1, "note": "first"},
        {"id": 2},
        {"id": 3, "note": "third"}
    ]));
    let result = render(
        "{% for it in items %}{{ it.id }}{% if it.note %}[{{ it.note }}]{% endif %} {% endfor %}",
        &ctx,
        "N/A",
    );
    assert_eq!(result, "1[first] 2 3[third] ");
}

// ── K. undefined_var_fallback template variable ───────────────────────────────

/// When `undefined_var_fallback` is set in the template, missing variables
/// render as that value.
#[test]
fn test_template_fallback_var_used_for_missing() {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    tera.add_raw_template(
        "tpl",
        "{% set undefined_var_fallback = 'N/A' %}{{ missing }}",
    )
    .unwrap();
    assert_eq!(tera.render("tpl", &Context::new()).unwrap(), "N/A");
}

/// When `undefined_var_fallback` is NOT set and loose rendering is on,
/// missing variables render as `[ERROR Rendering segment: Variable \`...\` not found]`.
#[test]
fn test_no_template_fallback_var_renders_error_tag() {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    tera.add_raw_template("tpl", "{{ missing }}").unwrap();
    let result = tera.render("tpl", &Context::new()).unwrap();
    assert_eq!(result, "[ERROR Rendering segment: Variable `missing` not found]");
}

/// `undefined_var_fallback` is visible inside for-loops (it lives in the
/// Origin frame which the lookup traverses).
#[test]
fn test_template_fallback_var_accessible_inside_for_loop() {
    let mut ctx = Context::new();
    ctx.insert("items", &vec!["a", "b"]);
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    tera.add_raw_template(
        "tpl",
        "{% set undefined_var_fallback = 'X' %}{% for item in items %}{{ item }}:{{ sep }} {% endfor %}",
    )
    .unwrap();
    let result = tera.render("tpl", &ctx).unwrap();
    assert_eq!(result, "a:X b:X ");
}

/// `undefined_var_fallback` set via `{% set_global %}` inside a block is also
/// found when the block is later rendered.
#[test]
fn test_template_fallback_var_set_before_usage() {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    // Fallback is set, then a missing variable is referenced.
    tera.add_raw_template(
        "tpl",
        "before:{{ x }} {% set undefined_var_fallback = 'FB' %} after:{{ y }}",
    )
    .unwrap();
    // `x` is referenced BEFORE the set — no fallback yet → [ERROR Rendering segment: ...]
    // `y` is referenced AFTER the set → "FB"
    let result = tera.render("tpl", &Context::new()).unwrap();
    assert!(
        result.starts_with("before:[ERROR Rendering segment:"),
        "unexpected: {result}"
    );
    assert!(result.ends_with("after:FB"), "unexpected: {result}");
}

/// `{% set %}` with a missing RHS resolves to `undefined_var_fallback`.
#[test]
fn test_set_from_missing_uses_template_fallback_var() {
    let mut tera = Tera::default();
    tera.set_loosely_render(true);
    tera.add_raw_template(
        "tpl",
        "{% set undefined_var_fallback = 'default' %}{% set x = missing %}{{ x }}",
    )
    .unwrap();
    assert_eq!(tera.render("tpl", &Context::new()).unwrap(), "default");
}

/// Without `loosely_render` enabled, `undefined_var_fallback` in the template
/// has no special meaning — missing variables still abort rendering.
#[test]
fn test_template_fallback_var_ignored_in_strict_mode() {
    let mut tera = Tera::default();
    // loosely_render is false (default)
    tera.add_raw_template(
        "tpl",
        "{% set undefined_var_fallback = 'N/A' %}{{ missing }}",
    )
    .unwrap();
    assert!(tera.render("tpl", &Context::new()).is_err());
}
