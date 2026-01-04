//! Help-text parsing into option specs for probe planning.

use std::collections::HashMap;

const SINGLE_SPACE_SPLIT_MAX_LEN: usize = 72;

#[derive(Debug, Clone)]
pub struct HelpOption {
    pub option: String,
    pub binding: Option<BindingHint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueForm {
    Attached,
    Trailing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindingHint {
    pub optional: bool,
    pub form: ValueForm,
}

#[derive(Debug, Clone)]
struct OptionRow {
    spec_segment: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OptionToken {
    raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArgToken {
    optional: bool,
    source: ArgSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArgSource {
    Attached,
    Trailing,
}

#[derive(Debug, Clone)]
enum SpecToken {
    Option(OptionToken),
    Arg(ArgToken),
    Separator,
}

#[derive(Debug, Clone)]
struct OptionSpec {
    options: Vec<OptionToken>,
    arg: Option<ArgSpec>,
}

#[derive(Debug, Clone)]
enum ArgSpec {
    Required { source: ArgSource },
    Optional { source: ArgSource },
}

/// Extract options and binding hints from help output.
pub fn extract_help_options(content: &str) -> Vec<HelpOption> {
    let mut options: Vec<HelpOption> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for row in detect_option_rows(content, looks_like_option_table) {
        let tokens = tokenize_spec(&row.spec_segment);
        let Some(spec) = parse_option_spec(&tokens) else {
            continue;
        };
        if spec.options.is_empty() {
            continue;
        }

        let binding = spec.arg.map(|arg| match arg {
            ArgSpec::Required { source, .. } => BindingHint {
                optional: false,
                form: source.into(),
            },
            ArgSpec::Optional { source, .. } => BindingHint {
                optional: true,
                form: source.into(),
            },
        });

        for option in spec.options.iter().map(|opt| opt.raw.clone()) {
            match index.get(&option).copied() {
                Some(idx) => {
                    let entry = &mut options[idx];
                    entry.binding = merge_binding_hint(entry.binding, binding);
                }
                None => {
                    let entry = HelpOption {
                        option: option.clone(),
                        binding,
                    };
                    index.insert(option, options.len());
                    options.push(entry);
                }
            }
        }
    }

    options
}

fn merge_binding_hint(existing: Option<BindingHint>, incoming: Option<BindingHint>) -> Option<BindingHint> {
    match (existing, incoming) {
        (None, other) => other,
        (Some(current), None) => Some(current),
        (Some(current), Some(next)) => {
            if current.form == ValueForm::Trailing && next.form == ValueForm::Attached {
                Some(next)
            } else {
                Some(current)
            }
        }
    }
}

impl From<ArgSource> for ValueForm {
    fn from(source: ArgSource) -> Self {
        match source {
            ArgSource::Attached => ValueForm::Attached,
            ArgSource::Trailing => ValueForm::Trailing,
        }
    }
}

// Extract rows that appear to be option-table entries.
fn detect_option_rows<F>(content: &str, line_selector: F) -> Vec<OptionRow>
where
    F: Fn(&str) -> bool,
{
    let mut rows = Vec::new();
    for line in content.lines() {
        if !line_selector(line) {
            continue;
        }
        let spec = option_spec_segment(line);
        if spec.is_empty() {
            continue;
        }
        rows.push(OptionRow {
            spec_segment: spec.to_string(),
        });
    }
    rows
}

// Return true when a line looks like the start of an option spec row.
fn looks_like_option_table(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with('-') && !trimmed.starts_with("---")
}

// Split a line into the "spec" segment (options + args) and description.
fn option_spec_segment(line: &str) -> &str {
    let trimmed = line.trim();
    split_on_double_space_index(trimmed)
        .or_else(|| split_on_single_space_fallback(trimmed))
        .map(|idx| trimmed[..idx].trim_end())
        .unwrap_or(trimmed)
}

fn split_on_double_space_index(line: &str) -> Option<usize> {
    line.as_bytes()
        .windows(2)
        .position(|pair| is_whitespace(pair[0]) && is_whitespace(pair[1]))
}

fn split_on_single_space_fallback(line: &str) -> Option<usize> {
    if line.len() > SINGLE_SPACE_SPLIT_MAX_LEN {
        return None;
    }
    let mut saw_option = false;
    let mut spec_tokens = 0;
    let mut non_spec_tokens = 0;
    let mut split_at = None;

    for (start, end) in token_spans(line) {
        let token = &line[start..end];
        let is_option = looks_like_option_token(token);
        if is_option {
            saw_option = true;
        }
        let is_spec = is_option || looks_like_arg_token(token) || looks_like_separator_token(token);
        if is_spec {
            spec_tokens += 1;
        } else {
            non_spec_tokens += 1;
            if saw_option && split_at.is_none() {
                split_at = Some(start);
            }
        }
    }

    if !saw_option {
        return None;
    }
    let split_at = split_at?;
    if spec_tokens >= non_spec_tokens {
        Some(split_at)
    } else {
        None
    }
}

fn token_spans(line: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut iter = line.char_indices().peekable();
    std::iter::from_fn(move || {
        while let Some((start, ch)) = iter.next() {
            if ch == ' ' || ch == '\t' {
                continue;
            }
            while let Some(&(_, next)) = iter.peek() {
                if next == ' ' || next == '\t' {
                    break;
                }
                iter.next();
            }
            let end = iter.peek().map(|(idx, _)| *idx).unwrap_or(line.len());
            return Some((start, end));
        }
        None
    })
}

fn is_whitespace(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn tokenize_spec(spec: &str) -> Vec<SpecToken> {
    let mut tokens = Vec::new();
    for word in spec.split_whitespace() {
        tokenize_word(word, &mut tokens);
    }
    tokens
}

fn tokenize_word(word: &str, tokens: &mut Vec<SpecToken>) {
    let mut segment = String::new();
    for ch in word.chars() {
        match ch {
            ',' | ';' => {
                flush_spec_segment(&mut segment, tokens);
                tokens.push(SpecToken::Separator);
            }
            ':' => {
                flush_spec_segment(&mut segment, tokens);
            }
            _ => segment.push(ch),
        }
    }
    flush_spec_segment(&mut segment, tokens);
}

fn flush_spec_segment(segment: &mut String, tokens: &mut Vec<SpecToken>) {
    if segment.is_empty() {
        return;
    }
    if let Some((option, arg)) = parse_option_segment(segment) {
        tokens.push(SpecToken::Option(option));
        if let Some(arg) = arg {
            tokens.push(SpecToken::Arg(arg));
        }
        return;
    }
    if let Some(arg) = parse_trailing_arg_segment(segment) {
        tokens.push(SpecToken::Arg(arg));
    }
}

fn parse_option_spec(tokens: &[SpecToken]) -> Option<OptionSpec> {
    let mut options = Vec::new();
    let mut arg: Option<ArgSpec> = None;

    for token in tokens {
        match token {
            SpecToken::Option(option) => options.push(option.clone()),
            SpecToken::Arg(arg_token) => {
                let candidate = ArgSpec::from_token(arg_token);
                arg = match arg {
                    None => Some(candidate),
                    Some(existing) => Some(prefer_arg_spec(existing, candidate)),
                };
            }
            SpecToken::Separator => {}
        }
    }

    if options.is_empty() {
        None
    } else {
        Some(OptionSpec { options, arg })
    }
}

fn prefer_arg_spec(existing: ArgSpec, candidate: ArgSpec) -> ArgSpec {
    if existing.source() == ArgSource::Trailing && candidate.source() == ArgSource::Attached {
        candidate
    } else {
        existing
    }
}

impl ArgSpec {
    fn from_token(token: &ArgToken) -> Self {
        if token.optional {
            ArgSpec::Optional {
                source: token.source,
            }
        } else {
            ArgSpec::Required {
                source: token.source,
            }
        }
    }

    fn source(&self) -> ArgSource {
        match self {
            ArgSpec::Required { source, .. } | ArgSpec::Optional { source, .. } => *source,
        }
    }
}

fn parse_option_segment(segment: &str) -> Option<(OptionToken, Option<ArgToken>)> {
    if let Some(parsed) = parse_long_option_segment(segment) {
        return Some(parsed);
    }
    parse_short_option_segment(segment)
}

fn parse_long_option_segment(segment: &str) -> Option<(OptionToken, Option<ArgToken>)> {
    if !segment.starts_with("--") {
        return None;
    }
    if segment.len() <= 2 {
        return None;
    }

    let (opt_part, arg_form) = split_attached_arg_form(segment)?;
    let name = &opt_part[2..];
    if name.is_empty() {
        return None;
    }
    let mut chars = name.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphanumeric() {
        return None;
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }

    Some((
        OptionToken {
            raw: opt_part.to_string(),
        },
        arg_form,
    ))
}

fn parse_short_option_segment(segment: &str) -> Option<(OptionToken, Option<ArgToken>)> {
    if !segment.starts_with('-') || segment.starts_with("--") {
        return None;
    }
    if segment.len() < 2 {
        return None;
    }

    let (opt_part, arg_form) = split_attached_arg_form(segment)?;
    let name = &opt_part[1..];
    if name.len() != 1 {
        return None;
    }
    let ch = name.chars().next()?;
    if !ch.is_ascii_alphanumeric() {
        return None;
    }

    Some((
        OptionToken {
            raw: opt_part.to_string(),
        },
        arg_form,
    ))
}

fn split_attached_arg_form(token: &str) -> Option<(&str, Option<ArgToken>)> {
    if let Some(idx) = token.find("[=") {
        if token.ends_with(']') {
            let opt_part = &token[..idx];
            let arg = &token[idx + 2..token.len() - 1];
            if arg.is_empty() {
                return None;
            }
            return Some((
                opt_part,
                Some(ArgToken {
                    optional: true,
                    source: ArgSource::Attached,
                }),
            ));
        }
    }

    if let Some(idx) = token.find('=') {
        let opt_part = &token[..idx];
        let arg = &token[idx + 1..];
        if arg.is_empty() {
            return None;
        }
        return Some((
            opt_part,
            Some(ArgToken {
                optional: false,
                source: ArgSource::Attached,
            }),
        ));
    }

    Some((token, None))
}

fn parse_trailing_arg_segment(segment: &str) -> Option<ArgToken> {
    let optional = classify_arg_token(segment)?;
    Some(ArgToken {
        optional,
        source: ArgSource::Trailing,
    })
}

fn classify_arg_token(token: &str) -> Option<bool> {
    if token.is_empty() {
        return None;
    }
    if let Some(inner) = token
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        if inner.is_empty() {
            return None;
        }
        return Some(true);
    }
    if let Some(inner) = token
        .strip_prefix('<')
        .and_then(|rest| rest.strip_suffix('>'))
    {
        if inner.is_empty() {
            return None;
        }
        return Some(false);
    }
    if is_upper_placeholder(token) {
        return Some(false);
    }
    None
}

fn is_upper_placeholder(token: &str) -> bool {
    let mut has_alpha = false;
    for ch in token.chars() {
        if ch.is_ascii_uppercase() {
            has_alpha = true;
        } else if ch.is_ascii_digit() || ch == '-' || ch == '_' {
            continue;
        } else {
            return false;
        }
    }
    has_alpha
}

fn trim_token_punct(token: &str) -> &str {
    token.trim_end_matches(|c: char| matches!(c, ',' | ';' | ':'))
}

fn looks_like_option_token(token: &str) -> bool {
    let trimmed = trim_token_punct(token);
    parse_option_segment(trimmed).is_some()
}

fn looks_like_arg_token(token: &str) -> bool {
    let trimmed = trim_token_punct(token);
    classify_arg_token(trimmed).is_some()
}

fn looks_like_separator_token(token: &str) -> bool {
    matches!(token, "," | ";")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_options_from_row() {
        let content = "  -a, --all  include dotfiles\n";
        let options = extract_help_options(content);
        let mut names: Vec<String> = options.into_iter().map(|opt| opt.option).collect();
        names.sort();
        assert_eq!(names, vec!["--all", "-a"]);
    }

    #[test]
    fn extracts_binding_hint_attached_optional() {
        let content = "  --color[=WHEN]  colorize output\n";
        let options = extract_help_options(content);
        let hint = options[0].binding.expect("binding hint");
        assert!(hint.optional);
        assert_eq!(hint.form, ValueForm::Attached);
    }

    #[test]
    fn ignores_non_option_lines() {
        let content = "Examples:\n  ls --color=auto\n";
        let options = extract_help_options(content);
        assert!(options.is_empty());
    }
}
