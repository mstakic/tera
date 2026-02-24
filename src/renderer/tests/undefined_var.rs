/// Tests for `Tera::set_undefined_variable_value` — the lenient rendering mode
/// where variables that are missing from the context render as a configured
/// fallback string instead of failing.
///
/// The tests are grouped into:
///   A. Basic VariableBlock output — what works
///   B. Filters on missing variables
///   C. Conditionals (if / not / and / or)
///   D. Testers (is defined / is undefined / is string / …)
///   E. Things that STILL fail despite the fallback being set
///   F. Subtle / surprising rendering behaviours
///   G. Configuration lifecycle
use serde_json::json;
// Import std::error::Error as a trait to make `.source()` available.
use std::error::Error as StdError;

use crate::context::Context;
use crate::errors::ErrorKind;
use crate::tera::Tera;

// ── helpers ──────────────────────────────────────────────────────────────────

fn tera_with_fallback(fallback: &str) -> Tera {
    let mut tera = Tera::default();
    tera.set_undefined_variable_value(Some(fallback.to_string()));
    tera
}

fn render(template: &str, ctx: &Context, fallback: &str) -> String {
    let mut tera = tera_with_fallback(fallback);
    tera.add_raw_template("tpl", template).unwrap();
    tera.render("tpl", ctx).unwrap()
}

fn render_err(template: &str, ctx: &Context, fallback: &str) -> crate::errors::Error {
    let mut tera = tera_with_fallback(fallback);
    tera.add_raw_template("tpl", template).unwrap();
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
    // undefined_variable_value kicks in, so it wins.
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
    // The ".html" suffix enables autoescaping.
    tera.add_raw_template("tpl.html", "{{ missing }}").unwrap();
    tera.set_undefined_variable_value(Some("<b>N/A</b>".to_string()));

    let result = tera.render("tpl.html", &Context::new()).unwrap();
    // Raw HTML is output as-is; no escaping applied to the fallback.
    assert_eq!(result, "<b>N/A</b>");
}

/// Defined variables ARE still escaped in autoescape mode; only the fallback
/// bypasses the escape function.
#[test]
fn test_defined_variable_still_escaped_in_autoescape_context() {
    let mut tera = Tera::default();
    tera.add_raw_template("tpl.html", "{{ xss }}{{ missing }}").unwrap();
    tera.set_undefined_variable_value(Some("N/A".to_string()));

    let mut ctx = Context::new();
    ctx.insert("xss", &"<script>");

    let result = tera.render("tpl.html", &ctx).unwrap();
    assert_eq!(result, "&lt;script&gt;N/A");
}

// ── G. Configuration lifecycle ────────────────────────────────────────────────

/// Disabling the fallback (passing None) restores strict error behaviour.
#[test]
fn test_disable_fallback_restores_strict_mode() {
    let mut tera = Tera::default();
    tera.add_raw_template("tpl", "{{ missing }}").unwrap();

    tera.set_undefined_variable_value(Some("N/A".to_string()));
    assert_eq!(tera.render("tpl", &Context::new()).unwrap(), "N/A");

    tera.set_undefined_variable_value(None);
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

/// Fallback is per-Tera-instance; two instances can have different fallbacks.
#[test]
fn test_fallback_is_per_instance() {
    let mut tera_na = Tera::default();
    tera_na.add_raw_template("tpl", "{{ missing }}").unwrap();
    tera_na.set_undefined_variable_value(Some("N/A".to_string()));

    let mut tera_unk = Tera::default();
    tera_unk.add_raw_template("tpl", "{{ missing }}").unwrap();
    tera_unk.set_undefined_variable_value(Some("UNKNOWN".to_string()));

    assert_eq!(tera_na.render("tpl", &Context::new()).unwrap(), "N/A");
    assert_eq!(tera_unk.render("tpl", &Context::new()).unwrap(), "UNKNOWN");
}
