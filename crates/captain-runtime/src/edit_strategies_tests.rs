use super::*;

#[test]
fn s1_simple_exact_match() {
    let r = try_edit("let x = 1;\n", "let x = 1;", "let x = 2;", false).unwrap();
    assert_eq!(r.strategy, "simple");
    assert_eq!(r.new_content, "let x = 2;\n");
    assert_eq!(r.replacements, 1);
}

#[test]
fn s1_simple_rejects_ambiguous() {
    let result = try_edit("foo\nfoo\n", "foo", "bar", false);
    assert!(
        result.is_err(),
        "expected error on ambiguous, got {result:?}"
    );
}

#[test]
fn s2_line_trimmed_handles_trailing_spaces() {
    let content = "fn a() {\n    let x = 1;   \n}\n";
    let old = "fn a() {\n    let x = 1;\n}";
    let new = "fn a() {\n    let x = 99;\n}";
    let r = try_edit(content, old, new, false).unwrap();
    assert_eq!(r.strategy, "line_trimmed");
    assert!(r.new_content.contains("let x = 99;"));
}

#[test]
fn s3_block_anchor_replaces_middle_blob() {
    let content = "open\n  garbage_line_a\n  garbage_line_b\n  garbage_line_c\nclose\n";
    let old = "open\n  XXX\n  YYY\nclose";
    let new = "open\n  fresh\nclose";
    let r = try_edit(content, old, new, false).unwrap();
    assert_eq!(r.strategy, "block_anchor");
    assert!(r.new_content.contains("fresh"));
    assert!(!r.new_content.contains("garbage"));
}

#[test]
fn s4_whitespace_normalized_collapses_runs() {
    let content = "let   x    =   1;\n";
    let old = "let x = 1;";
    let new = "let x = 2;";
    let r = try_edit(content, old, new, false).unwrap();
    assert_eq!(r.strategy, "whitespace_normalized");
    assert!(r.new_content.contains("let x = 2;"));
}

#[test]
fn s5_indentation_flexible_handles_indent_diff() {
    let content = "    foo();\n    bar();\n";
    let old = "  foo();\n  bar();";
    let new = "  baz();\n  qux();";
    let r = try_edit(content, old, new, false).unwrap();
    assert!(r.new_content.contains("baz();"));
    assert!(r.new_content.contains("qux();"));
    assert!(!r.new_content.contains("foo();"));
}

#[test]
fn s5_direct_indentation_flexible_unit() {
    let r = indentation_flexible_replacer(
        "        let x = 1;\n        let y = 2;\n",
        "  let x = 1;\n  let y = 2;",
        "  let x = 9;\n  let y = 8;",
    );
    let out = r.expect("indentation_flexible should match leading-only differences");
    assert!(out.contains("let x = 9;"));
    assert!(out.contains("let y = 8;"));
}

#[test]
fn s6_escape_normalized_unescapes_backslash_n() {
    let content = "first\nsecond\n";
    let old = "first\\nsecond";
    let new = "FIRST\nSECOND";
    let r = try_edit(content, old, new, false).unwrap();
    assert_eq!(r.strategy, "escape_normalized");
    assert!(r.new_content.contains("FIRST"));
    assert!(r.new_content.contains("SECOND"));
}

#[test]
fn s7_trimmed_boundary_strips_extra_whitespace() {
    let content = "fn answer() -> u32 { 42 }\n";
    let old = "\n\n  fn answer() -> u32 { 42 }  \n";
    let new = "fn answer() -> u32 { 41 }";
    let r = try_edit(content, old, new, false).unwrap();
    assert!(r.new_content.contains("41"));
    assert!(!r.new_content.contains("42"));
}

#[test]
fn s7_direct_trimmed_boundary_unit() {
    let out = trimmed_boundary_replacer("ab cd ef\n", "  cd  ", "ZZ")
        .expect("should trim boundary then match `cd`");
    assert_eq!(out, "ab ZZ ef\n");
}

#[test]
fn s8_context_aware_anchors_on_head_and_tail() {
    let content = "head_a\nhead_b\nm1\nm2\nm3\nm4\ntail_a\ntail_b\n";
    let old = "head_a\nhead_b\nDIFFERENT\nDIFFERENT\nDIFFERENT\nDIFFERENT\ntail_a\ntail_b";
    let new = "head_a\nhead_b\nfresh\ntail_a\ntail_b";
    let r = try_edit(content, old, new, false).unwrap();
    assert!(r.new_content.contains("fresh"));
    assert!(!r.new_content.contains("m1"));
}

#[test]
fn s8_direct_context_aware_unit() {
    let content = "head1\nhead2\nold_a\nold_b\nold_c\ntail1\ntail2\n";
    let old = "head1\nhead2\nm1\nm2\nm3\ntail1\ntail2";
    let new = "head1\nhead2\nNEW\ntail1\ntail2";
    let out = context_aware_replacer(content, old, new)
        .expect("context_aware should anchor on 2+2 lines");
    assert!(out.contains("NEW"));
    assert!(!out.contains("old_a"));
}

#[test]
fn s9_multi_occurrence_replaces_all() {
    let content = "TODO fix\nTODO refactor\nTODO test\n";
    let r = try_edit(content, "TODO", "DONE", true).unwrap();
    assert_eq!(r.strategy, "multi_occurrence");
    assert_eq!(r.replacements, 3);
    assert!(!r.new_content.contains("TODO"));
    assert_eq!(r.new_content.matches("DONE").count(), 3);
}

#[test]
fn errors_on_empty_old_string() {
    let r = try_edit("hello", "", "x", false);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("cannot be empty"));
}

#[test]
fn errors_on_no_op() {
    let r = try_edit("hello", "x", "x", false);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("no-op"));
}

#[test]
fn errors_on_no_match() {
    let r = try_edit("hello world", "absent", "x", false);
    assert!(r.is_err());
    let msg = r.unwrap_err();
    assert!(msg.contains("No fallback strategy matched"), "got: {msg}");
}

#[test]
fn replace_all_errors_when_pattern_absent() {
    let r = try_edit("hello", "absent", "x", true);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("not found"));
}

#[test]
fn preserves_trailing_newline_policy_when_present() {
    let r = try_edit("a\nb\n", "b", "B", false).unwrap();
    assert_eq!(r.new_content, "a\nB\n");
}

#[test]
fn preserves_no_trailing_newline_policy() {
    let content = "fn a() {\n    let x = 1;\n}";
    let old = "fn a() {\n    let x = 1;\n}";
    let new = "fn a() {\n    let x = 9;\n}";
    let r = try_edit(content, old, new, false).unwrap();
    assert!(!r.new_content.ends_with('\n'));
    assert!(r.new_content.contains("let x = 9;"));
}
