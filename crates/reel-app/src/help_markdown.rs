//! Bundled help topics are markdown. The Slint runtime parser (`parse_markdown`, same as `@markdown`)
//! only supports a subset of CommonMark (no ATX headings, tables, or fenced code blocks). We **lower**
//! full markdown to that subset, then convert to [`StyledText`](slint::private_unstable_api::re_exports::StyledText).

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use slint::private_unstable_api::re_exports::{parse_markdown, StyledText};

struct ListCtx {
    ordered: bool,
    /// Next index to print for ordered lists (`1.`, `2.`, …).
    next: u64,
}

/// Map full markdown to a string Slint’s markdown parser accepts (bold, lists, links, inline code, …).
fn lower_for_slint(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(md, opts);
    let mut out = String::new();
    let mut list_stack: Vec<ListCtx> = Vec::new();
    let mut link_url: Vec<String> = Vec::new();
    let mut table_row_first_cell = true;
    let mut in_table_row = false;

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {}
                Tag::Heading { .. } => out.push_str("**"),
                Tag::BlockQuote(_) => out.push_str("\n\n"),
                Tag::CodeBlock(_) => out.push_str("\n\n"),
                Tag::HtmlBlock => out.push_str("\n\n"),
                Tag::List(None) => {
                    list_stack.push(ListCtx {
                        ordered: false,
                        next: 0,
                    });
                }
                Tag::List(Some(start)) => {
                    list_stack.push(ListCtx {
                        ordered: true,
                        next: start,
                    });
                }
                Tag::Item => {
                    if let Some(ctx) = list_stack.last_mut() {
                        if ctx.ordered {
                            let n = ctx.next;
                            ctx.next = ctx.next.saturating_add(1);
                            use std::fmt::Write;
                            let _ = write!(out, "{}. ", n);
                        } else {
                            out.push_str("- ");
                        }
                    }
                }
                Tag::FootnoteDefinition(_) => out.push_str("\n\n["),
                Tag::DefinitionList | Tag::DefinitionListTitle | Tag::DefinitionListDefinition => {
                    out.push('\n');
                }
                Tag::Table(_) => out.push_str("\n\n"),
                Tag::TableHead => {}
                Tag::TableRow => {
                    in_table_row = true;
                    table_row_first_cell = true;
                    out.push('\n');
                }
                Tag::TableCell => {
                    if in_table_row && !table_row_first_cell {
                        out.push_str(" · ");
                    }
                    table_row_first_cell = false;
                }
                Tag::Emphasis => out.push('*'),
                Tag::Strong => out.push_str("**"),
                Tag::Strikethrough => out.push_str("~~"),
                Tag::Superscript => out.push('^'),
                Tag::Subscript => out.push('~'),
                Tag::Link { dest_url, .. } => {
                    link_url.push(dest_url.to_string());
                    out.push('[');
                }
                Tag::Image { dest_url, .. } => {
                    link_url.push(dest_url.to_string());
                    out.push_str("![");
                }
                Tag::MetadataBlock(_) => out.push_str("\n\n"),
            },

            Event::End(end) => match end {
                TagEnd::Paragraph => out.push_str("\n\n"),
                TagEnd::Heading(_) => out.push_str("**\n\n"),
                TagEnd::BlockQuote(_) => out.push_str("\n\n"),
                TagEnd::CodeBlock => out.push_str("\n\n"),
                TagEnd::HtmlBlock => out.push_str("\n\n"),
                TagEnd::List(_) => {
                    list_stack.pop();
                    out.push('\n');
                }
                TagEnd::Item => out.push('\n'),
                TagEnd::FootnoteDefinition => out.push_str("]\n\n"),
                TagEnd::DefinitionList
                | TagEnd::DefinitionListTitle
                | TagEnd::DefinitionListDefinition => {
                    out.push('\n');
                }
                TagEnd::Table => out.push_str("\n\n"),
                TagEnd::TableHead => {}
                TagEnd::TableRow => {
                    in_table_row = false;
                }
                TagEnd::TableCell => {}
                TagEnd::Emphasis => out.push('*'),
                TagEnd::Strong => out.push_str("**"),
                TagEnd::Strikethrough => out.push_str("~~"),
                TagEnd::Superscript => {}
                TagEnd::Subscript => {}
                TagEnd::Link => {
                    if let Some(url) = link_url.pop() {
                        out.push(']');
                        out.push('(');
                        out.push_str(&url);
                        out.push(')');
                    }
                }
                TagEnd::Image => {
                    if let Some(url) = link_url.pop() {
                        out.push(']');
                        out.push('(');
                        out.push_str(&url);
                        out.push(')');
                    }
                }
                TagEnd::MetadataBlock(_) => out.push_str("\n\n"),
            },

            Event::Text(t) => out.push_str(t.as_ref()),
            Event::Code(t) => {
                out.push('`');
                out.push_str(t.as_ref());
                out.push('`');
            }
            Event::Html(t) | Event::InlineHtml(t) => {
                out.push_str(&flatten_inline_html(t.as_ref()));
            }
            Event::InlineMath(t) | Event::DisplayMath(t) => {
                out.push('`');
                out.push_str(t.as_ref());
                out.push('`');
            }
            Event::FootnoteReference(label) => {
                out.push_str("[^");
                out.push_str(label.as_ref());
                out.push(']');
            }
            Event::SoftBreak => out.push('\n'),
            Event::HardBreak => out.push_str("  \n"),
            Event::Rule => out.push_str("\n\n────────\n\n"),
            Event::TaskListMarker(done) => {
                if done {
                    out.push_str("[x] ");
                } else {
                    out.push_str("[ ] ");
                }
            }
        }
    }

    out
}

fn flatten_inline_html(raw: &str) -> String {
    let s = raw
        .replace("<br>", "\n")
        .replace("<br/>", "\n")
        .replace("<br />", "\n");
    // Drop other tags conservatively (help docs rarely use rich inline HTML).
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Convert bundled help markdown to Slint styled text for [`crate::HelpWindow`].
pub fn markdown_to_styled(md: &str) -> StyledText {
    let lowered = lower_for_slint(md);
    parse_markdown::<StyledText>(&lowered, &[])
}

#[cfg(test)]
mod tests {
    use slint::private_unstable_api::re_exports::string_to_styled_text;

    use super::*;

    #[test]
    fn bold_and_code_parse_as_styled_not_plain_wrapping() {
        let sample = "**emphasis** and `code`";
        let styled = markdown_to_styled(sample);
        let plain_wrapped = string_to_styled_text(sample.into());
        assert_ne!(
            styled, plain_wrapped,
            "help must use Slint markdown parsing so ** and ` are not shown literally"
        );
    }

    #[test]
    fn heading_lowered_then_parses() {
        let md = "# Title\n\nBody **x**.\n";
        let lowered = lower_for_slint(md);
        assert!(
            lowered.contains("**Title**"),
            "expected ATX heading lowered to bold: {lowered:?}"
        );
        markdown_to_styled(md);
    }

    #[test]
    fn bundled_overview_help_parses_without_panic() {
        let md = crate::shell::HelpDoc::Overview.markdown();
        markdown_to_styled(md);
    }

    #[test]
    fn all_bundled_help_topics_parse_as_styled() {
        use crate::shell::HelpDoc;
        for doc in [
            HelpDoc::About,
            HelpDoc::Overview,
            HelpDoc::Features,
            HelpDoc::Keyboard,
            HelpDoc::MediaFormats,
            HelpDoc::SupportedFormats,
            HelpDoc::Cli,
            HelpDoc::ExternalAi,
            HelpDoc::Developers,
            HelpDoc::Agents,
            HelpDoc::PhasesUi,
        ] {
            markdown_to_styled(doc.markdown());
        }
    }

    #[test]
    fn headings_lists_and_table_snippet_parse() {
        let md = r#"# Title

Intro **bold** and *italic*.

- one
- two

| A | B |
|---|---|
| x | y |
"#;
        markdown_to_styled(md);
    }

    #[test]
    fn br_in_html_becomes_newline() {
        let lowered = lower_for_slint("a<br>b");
        assert!(lowered.starts_with("a\nb"), "got {lowered:?}");
    }
}
