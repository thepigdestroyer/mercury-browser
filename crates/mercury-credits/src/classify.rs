// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Which licences a text is, judged by its distinctive wording, and the text handling around it
//! (what makes two texts "the same", where a copyright line goes into a standard text).
//!
//! The wording is compared after folding case, quotes and white space, so a licence wrapped at
//! another column, or in a Markdown file, is still found. A text may be several licences at once
//! (aws-lc's `LICENSE` holds four); every one found is reported.

/// Lowercases, turns typographic quotes and dashes into plain ones, drops the comment markers
/// (`//`, `#`, `*`, `>`) at the start of lines and collapses all white space, so that line
/// breaks, indentation and a licence kept in a source comment do not matter.
pub fn fold(text: &str) -> String {
    let mut folded = String::with_capacity(text.len());
    for line in text.lines() {
        let line = line
            .trim_start()
            .trim_start_matches(['/', '#', '*', '>', ';']);
        // A line break is a space.
        let mut pending_space = !folded.is_empty();
        for c in line.chars() {
            let c = match c {
                '\u{2018}' | '\u{2019}' | '\u{0060}' => '\'',
                '\u{201C}' | '\u{201D}' => '"',
                '\u{2013}' | '\u{2014}' | '\u{2010}' | '\u{2011}' => '-',
                c => c,
            };
            if c.is_whitespace() || c == '\u{feff}' {
                pending_space = !folded.is_empty();
            } else {
                if pending_space {
                    folded.push(' ');
                    pending_space = false;
                }
                folded.extend(c.to_lowercase());
            }
        }
    }
    folded
}

/// The first `count` characters of a folded text: where licences put their title.
fn head(folded: &str, count: usize) -> &str {
    match folded.char_indices().nth(count) {
        Some((end, _)) => &folded[..end],
        None => folded,
    }
}

fn has(folded: &str, phrase: &str) -> bool {
    folded.contains(phrase)
}

/// Splits `folded` at every occurrence of `marker`, each part starting at its marker.
fn segments<'a>(folded: &'a str, marker: &str) -> Vec<&'a str> {
    let starts: Vec<usize> = folded
        .match_indices(marker)
        .map(|(start, _)| start)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(folded.len());
            &folded[*start..end]
        })
        .collect()
}

fn bsd_variant(segment: &str) -> Option<&'static str> {
    if !(has(segment, "redistributions of source code must retain")
        && has(segment, "redistributions in binary form must reproduce"))
    {
        return None;
    }
    let advertising = has(segment, "all advertising materials mentioning features");
    let endorse = has(
        segment,
        "endorse or promote products derived from this software",
    ) || has(segment, "may be used to endorse or promote");
    Some(match (advertising, endorse) {
        (true, _) => "BSD-4-Clause",
        (false, true) => "BSD-3-Clause",
        (false, false) => "BSD-2-Clause",
    })
}

/// Every licence the text contains, sorted and without repeats. Empty when the wording is not one
/// of the licences the tool knows.
pub fn detect(text: &str) -> Vec<String> {
    let folded = fold(text);
    let title = head(&folded, 400);
    let mut found: Vec<&'static str> = Vec::new();

    if has(
        &folded,
        "terms and conditions for use, reproduction, and distribution",
    ) && has(&folded, "apache license")
        && has(&folded, "version 2.0, january 2004")
    {
        found.push("Apache-2.0");
        if has(&folded, "llvm exceptions to the apache 2.0 license") {
            found.push("Apache-2.0 WITH LLVM-exception");
        }
    }

    let mit_grant = has(
        &folded,
        "permission is hereby granted, free of charge, to any person obtaining a copy of",
    ) && (has(&folded, "to deal in the software without restriction")
        || has(&folded, "to deal in the materials without restriction"));
    if mit_grant {
        if has(
            &folded,
            "the above copyright notice and this permission notice shall be included in all copies or substantial portions of the",
        ) {
            found.push("MIT");
        } else {
            found.push("MIT-0");
        }
    }

    for segment in segments(&folded, "redistribution and use in source and binary forms") {
        if let Some(id) = bsd_variant(segment) {
            found.push(id);
        }
    }

    if has(
        &folded,
        "permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted",
    ) || has(
        &folded,
        "permission to use, copy, modify, and distribute this software for any purpose with or without fee is hereby granted",
    ) {
        if has(
            &folded,
            "provided that the above copyright notice and this permission notice appear in all copies",
        ) {
            found.push("ISC");
        } else {
            found.push("0BSD");
        }
    }

    if (has(
        &folded,
        "provided 'as-is', without any express or implied warranty",
    ) || has(
        &folded,
        "provided \"as-is\", without any express or implied warranty",
    ) || has(
        &folded,
        "provided 'as is', without any express or implied warranty",
    ) || has(
        &folded,
        "provided \"as is\", without any express or implied warranty",
    )) && has(
        &folded,
        "altered source versions must be plainly marked as such",
    ) {
        found.push("Zlib");
    }

    if has(&folded, "mozilla public license version 2.0")
        || (has(&folded, "mozilla public license, version 2.0") && has(&folded, "1. definitions"))
    {
        found.push("MPL-2.0");
    }
    if has(&folded, "mozilla public license version 1.1") {
        found.push("MPL-1.1");
    }
    // HarfBuzz's "Old MIT" licence: its own wording of the MIT grant.
    if has(
        &folded,
        "permission is hereby granted, without written agreement and without license or royalty fees, to use, copy, modify, and distribute this software and its documentation",
    ) {
        found.push("MIT");
    }
    if has(title, "attribution-noderivatives 4.0 international") {
        found.push("CC-BY-ND-4.0");
    } else if has(title, "attribution-sharealike 4.0 international") {
        found.push("CC-BY-SA-4.0");
    } else if has(title, "attribution 4.0 international") {
        found.push("CC-BY-4.0");
    }
    if has(&folded, "unicode license v3") {
        found.push("Unicode-3.0");
    }
    if has(
        &folded,
        "unicode, inc. license agreement - data files and software",
    ) {
        found.push("Unicode-DFS-2016");
    }
    if has(&folded, "cc0 1.0 universal") && has(&folded, "creative commons") {
        found.push("CC0-1.0");
    }
    if has(
        &folded,
        "this is free and unencumbered software released into the public domain",
    ) {
        found.push("Unlicense");
    }
    if has(&folded, "boost software license - version 1.0") {
        found.push("BSL-1.0");
    }
    if has(&folded, "university of illinois/ncsa open source license") {
        found.push("NCSA");
    }
    if has(title, "community data license agreement")
        && has(title, "permissive")
        && has(title, "version 2.0")
    {
        found.push("CDLA-Permissive-2.0");
    }
    if has(
        title,
        "slint royalty-free desktop, mobile, and web applications license",
    ) && has(title, "version 2.0")
    {
        found.push("LicenseRef-Slint-Royalty-free-2.0");
    }
    if has(title, "slint software license") && has(title, "version 3.0") {
        found.push("LicenseRef-Slint-Software-3.0");
    }

    let gnu_title = head(&folded, 300);
    if has(gnu_title, "gnu affero general public license") {
        found.push("AGPL-3.0-only");
    } else if has(gnu_title, "gnu lesser general public license") {
        found.push(if has(gnu_title, "version 2.1") {
            "LGPL-2.1-only"
        } else {
            "LGPL-3.0-only"
        });
    } else if has(gnu_title, "gnu library general public license") {
        found.push("LGPL-2.0-only");
    } else if has(gnu_title, "gnu general public license") {
        found.push(
            if has(gnu_title, "version 2") && !has(gnu_title, "version 3") {
                "GPL-2.0-only"
            } else {
                "GPL-3.0-only"
            },
        );
    }

    found.sort_unstable();
    found.dedup();
    found.into_iter().map(str::to_owned).collect()
}

/// Wording that only a licence text (or a notice carrying one) has.
const GRANT_WORDS: &[&str] = &[
    "permission is hereby granted",
    "permission to use",
    "redistribution and use",
    "warranty",
    "warranties",
    "liable",
    "liability",
    "merchantability",
    "terms and conditions",
    "hereby grants",
    "grant of license",
];

/// How a text says which licences apply.
const STATEMENT_PHRASES: &[&str] = &[
    "licensed under",
    "dual-licensed",
    "dual licensed",
    "triple-licensed",
    "tri-licensed",
    "licensed only under",
    "released under",
    "available under",
    "is licensed",
    "are licensed",
];

/// Whether a text only says which licences apply (a bare SPDX expression, a path to another
/// file, an SPDX document, "dual-licensed under A and B", the Apache boilerplate notice) and
/// grants nothing itself.
pub fn is_statement(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let folded = fold(trimmed);
    if trimmed.len() < 200 && crate::spdx::parse(trimmed).is_ok() {
        return true;
    }
    if trimmed.len() < 200
        && !trimmed.contains(char::is_whitespace)
        && (folded.contains("licen") || folded.contains("copying"))
    {
        return true;
    }
    if folded.starts_with("spdxversion:") {
        return true;
    }
    // The Apache boilerplate notice ("Licensed under the Apache License, Version 2.0 (the
    // "License"); you may not use this file except...") and Mozilla's variant of it.
    if trimmed.len() < 3000
        && has(&folded, "licensed under the apache license")
        && (has(&folded, "you may obtain a copy of the license at")
            || has(&folded, "you may obtain copies of the licenses at")
            || has(&folded, "at your option"))
    {
        return true;
    }
    // "dual-licensed under A and B", "Licensed under X or Y at your option", with nothing that
    // grants a right or disclaims a warranty. A custom licence does not say it is "licensed
    // under" itself, so it stays unknown and gets a warning.
    folded.len() <= 8000
        && STATEMENT_PHRASES.iter().any(|phrase| has(&folded, phrase))
        && !GRANT_WORDS.iter().any(|word| has(&folded, word))
}

/// What a licence file turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    /// The text of these licences (several for a composite file).
    Licenses(Vec<String>),
    /// A statement of which licences apply, not a licence text.
    Statement,
    /// Wording the tool does not know.
    Unknown,
}

pub fn classify(text: &str) -> Class {
    let found = detect(text);
    if !found.is_empty() {
        Class::Licenses(found)
    } else if is_statement(text) {
        Class::Statement
    } else {
        Class::Unknown
    }
}

/// What makes two texts one block of the page: line ends as LF, trailing white space trimmed on
/// every line, blank lines at the start and the end removed.
pub fn normalize(text: &str) -> String {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim_end_matches([' ', '\t', '\r', '\u{c}']))
        .collect();
    while lines.first().is_some_and(|line| line.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    lines.join("\n")
}

const PLACEHOLDERS: &[&str] = &[
    "<year>",
    "[year]",
    "[yyyy]",
    "<copyright holders>",
    "<copyright holder>",
    "[name of copyright owner]",
    "<owner>",
    "<name of author>",
    "<organization>",
    "<insert copyright",
];

fn is_placeholder_line(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    let starts_like_a_copyright =
        lower.starts_with("copyright") || lower.starts_with("(c)") || lower.starts_with('\u{a9}');
    starts_like_a_copyright
        && PLACEHOLDERS
            .iter()
            .any(|placeholder| lower.contains(placeholder))
}

/// A standard licence text with `copyright` in it: the first line that holds a placeholder
/// (`<year>`, `<copyright holders>`, `[name of copyright owner]`...) is replaced by it, keeping
/// its indentation; a text without one gets the line above it.
pub fn with_copyright(standard: &str, copyright: &str) -> String {
    let standard = normalize(standard);
    let mut replaced = false;
    let lines: Vec<String> = standard
        .split('\n')
        .map(|line| {
            if !replaced && is_placeholder_line(line) {
                replaced = true;
                let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
                format!("{indent}{copyright}")
            } else {
                line.to_owned()
            }
        })
        .collect();
    if replaced {
        lines.join("\n")
    } else {
        format!("{copyright}\n\n{standard}")
    }
}
