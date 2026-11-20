// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! Which licence a text is, from its wording; and the text handling around it.
mod common;

use common::{APACHE, BSD3, ISC, MIT, MPL};
use mercury_credits::classify::{self, Class};
use std::path::Path;

fn ids(text: &str) -> Vec<String> {
    classify::detect(text)
}

fn list(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn the_common_licences_are_recognised_by_their_words() {
    assert_eq!(ids(MIT), list(&["MIT"]));
    assert_eq!(ids(APACHE), list(&["Apache-2.0"]));
    assert_eq!(ids(MPL), list(&["MPL-2.0"]));
    assert_eq!(ids(ISC), list(&["ISC"]));
    assert_eq!(ids(BSD3), list(&["BSD-3-Clause"]));
}

#[test]
fn bsd_two_and_three_clause_are_told_apart() {
    let two = BSD3
        .replace(
            "3. Neither the name of the copyright holder nor the names of its\n   contributors may be used to endorse or promote products derived from\n   this software without specific prior written permission.\n\n",
            "",
        );
    assert!(!two.contains("Neither"));
    assert_eq!(ids(&two), list(&["BSD-2-Clause"]));
    let four = BSD3.replace(
        "3. Neither",
        "3. All advertising materials mentioning features of this software must display the following acknowledgement.\n\n4. Neither",
    );
    assert_eq!(ids(&four), list(&["BSD-4-Clause"]));
}

#[test]
fn zlib_unlicense_boost_ncsa_cdla_unicode_and_cc0_are_recognised() {
    let zlib = "Copyright (c) 1995 the authors\n\nThis software is provided 'as-is', without any express or implied\nwarranty.  In no event will the authors be held liable for any damages\narising from the use of this software.\n\n2. Altered source versions must be plainly marked as such, and must not be\n   misrepresented as being the original software.\n";
    assert_eq!(ids(zlib), list(&["Zlib"]));
    let unlicense = "This is free and unencumbered software released into the public domain.\n\nAnyone is free to copy, modify, publish, use, compile, sell, or distribute this software.";
    assert_eq!(ids(unlicense), list(&["Unlicense"]));
    assert_eq!(
        ids(
            "Boost Software License - Version 1.0 - August 17th, 2003\n\nPermission is hereby granted"
        ),
        list(&["BSL-1.0"])
    );
    assert_eq!(
        ids(
            "University of Illinois/NCSA Open Source License\n\nCopyright (c) 2003-2019 University of Illinois at Urbana-Champaign."
        ),
        list(&["NCSA"])
    );
    assert_eq!(
        ids(
            "Community Data License Agreement - Permissive, Version 2.0\n\nThis CDLA-Permissive-2.0 ..."
        ),
        list(&["CDLA-Permissive-2.0"])
    );
    assert_eq!(
        ids("UNICODE LICENSE V3\n\nCOPYRIGHT AND PERMISSION NOTICE"),
        list(&["Unicode-3.0"])
    );
    assert_eq!(
        ids("UNICODE, INC. LICENSE AGREEMENT - DATA FILES AND SOFTWARE\n\nSee Terms of Use"),
        list(&["Unicode-DFS-2016"])
    );
    assert_eq!(
        ids(
            "Creative Commons Legal Code\n\nCC0 1.0 Universal\n\nCREATIVE COMMONS CORPORATION IS NOT A LAW FIRM"
        ),
        list(&["CC0-1.0"])
    );
}

#[test]
fn unicode_text_is_not_mistaken_for_mit() {
    let unicode = "UNICODE LICENSE V3\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of data files and any associated documentation (the \"Data Files\") or software and any associated documentation (the \"Software\") to deal in the Data Files or Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, and/or sell copies of the Data Files or Software.\n\nThe above copyright notice and this permission notice shall be included in all copies or substantial portions of the Data Files or Software.";
    assert_eq!(ids(unicode), list(&["Unicode-3.0"]));
}

#[test]
fn slint_terms_are_told_apart_by_their_title() {
    let royalty_free = "# Slint Royalty-free Desktop, Mobile, and Web Applications License\n\nVersion 2.0\n\n## Preamble\n";
    let software = "# Slint Software License\n\nVersion 3.0.5\n\n## Preamble\n";
    assert_eq!(
        ids(royalty_free),
        list(&["LicenseRef-Slint-Royalty-free-2.0"])
    );
    assert_eq!(ids(software), list(&["LicenseRef-Slint-Software-3.0"]));
}

#[test]
fn the_gnu_licences_are_recognised_by_their_heading() {
    assert_eq!(
        ids("GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007\n\nCopyright (C) 2007"),
        list(&["GPL-3.0-only"])
    );
    assert_eq!(
        ids("GNU GENERAL PUBLIC LICENSE\nVersion 2, June 1991\n"),
        list(&["GPL-2.0-only"])
    );
    assert_eq!(
        ids("GNU LESSER GENERAL PUBLIC LICENSE\nVersion 2.1, February 1999\n"),
        list(&["LGPL-2.1-only"])
    );
    assert_eq!(
        ids("GNU LESSER GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007\n"),
        list(&["LGPL-3.0-only"])
    );
    assert_eq!(
        ids("GNU AFFERO GENERAL PUBLIC LICENSE\nVersion 3, 19 November 2007\n"),
        list(&["AGPL-3.0-only"])
    );
    // The GPL text mentions the Affero licence in section 13; only the heading counts.
    let gpl = format!(
        "GNU GENERAL PUBLIC LICENSE\nVersion 3, 29 June 2007\n\n{}\n13. Use with the GNU Affero General Public License.",
        "x ".repeat(400)
    );
    assert_eq!(ids(&gpl), list(&["GPL-3.0-only"]));
}

#[test]
fn zero_clause_bsd_and_mit_zero_are_not_isc_and_mit() {
    let zero_bsd = "Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is hereby granted.\n\nTHE SOFTWARE IS PROVIDED \"AS IS\"";
    assert_eq!(ids(zero_bsd), list(&["0BSD"]));
    let mit_zero = MIT.replace(
        "The above copyright notice and this permission notice shall be included in all\ncopies or substantial portions of the Software.\n\n",
        "",
    );
    assert_eq!(ids(&mit_zero), list(&["MIT-0"]));
}

#[test]
fn the_creative_commons_licences_for_documentation_are_known() {
    assert_eq!(
        ids(
            "Creative Commons Attribution-NoDerivatives 4.0 International Public License\n\nBy exercising"
        ),
        list(&["CC-BY-ND-4.0"])
    );
    assert_eq!(
        ids("Attribution-ShareAlike 4.0 International\n\n====="),
        list(&["CC-BY-SA-4.0"])
    );
    assert_eq!(
        ids("Attribution 4.0 International\n\n====="),
        list(&["CC-BY-4.0"])
    );
}

#[test]
fn harfbuzz_old_mit_counts_as_mit() {
    let text = "HarfBuzz is licensed under the so-called \"Old MIT\" license.  Details follow.\n\nPermission is hereby granted, without written agreement and without\nlicense or royalty fees, to use, copy, modify, and distribute this\nsoftware and its documentation for any purpose, provided that the\nabove copyright notice and the following two paragraphs appear in\nall copies of this software.";
    assert_eq!(ids(text), list(&["MIT"]));
}

#[test]
fn a_composite_file_reports_every_licence_in_it() {
    let composite = format!("{ISC}\n\n----\n\n{APACHE}\n\n----\n\n{BSD3}\n\n----\n\n{MIT}");
    assert_eq!(
        ids(&composite),
        list(&["Apache-2.0", "BSD-3-Clause", "ISC", "MIT"])
    );
    // Two BSD texts of different kinds in one file are both found.
    let two_bsd = format!(
        "{}\n\n----\n\n{BSD3}",
        BSD3.replace(
            "3. Neither the name of the copyright holder nor the names of its\n   contributors may be used to endorse or promote products derived from\n   this software without specific prior written permission.\n\n",
            "",
        )
    );
    assert_eq!(ids(&two_bsd), list(&["BSD-2-Clause", "BSD-3-Clause"]));
}

#[test]
fn the_llvm_exception_is_a_licence_of_its_own() {
    let text = format!(
        "{APACHE}\n\n--- LLVM Exceptions to the Apache 2.0 License ----\n\nAs an exception, if you ..."
    );
    assert_eq!(
        ids(&text),
        list(&["Apache-2.0", "Apache-2.0 WITH LLVM-exception"])
    );
}

#[test]
fn wrapping_markdown_and_comment_markers_do_not_matter() {
    let wrapped = MIT
        .replace('\n', "\r\n")
        .replace("free of charge,", "free of charge,\r\n    ");
    assert_eq!(ids(&wrapped), list(&["MIT"]));
    let commented: String = ISC.lines().map(|line| format!("// {line}\n")).collect();
    assert_eq!(ids(&commented), list(&["ISC"]));
    let hashed: String = ISC.lines().map(|line| format!("# {line}\n")).collect();
    assert_eq!(ids(&hashed), list(&["ISC"]));
    let markdown = format!("# License\n\n> {}", MIT.replace('\n', "\n> "));
    assert_eq!(ids(&markdown), list(&["MIT"]));
    let curly = ISC.replace('"', "\u{201c}");
    assert_eq!(ids(&curly), list(&["ISC"]));
}

#[test]
fn text_that_is_not_a_licence_is_not_classified() {
    assert!(ids("").is_empty());
    assert!(ids("This crate is cool. Use it as you like.").is_empty());
    assert!(
        ids("Copyright 2020 Somebody. All rights reserved. Redistribution is not permitted.")
            .is_empty()
    );
}

#[test]
fn statements_that_only_name_licences_are_not_licence_texts() {
    for statement in [
        "MIT OR Apache-2.0",
        "MIT/Apache-2.0\n",
        "../../LICENSE-MIT",
        "LICENSE-APACHE",
        "This project is dual-licensed under the Unlicense and MIT licenses.\n\nYou may use this code under the terms of either license.\n",
        "Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or\nhttp://www.apache.org/licenses/LICENSE-2.0> or the MIT license\n<LICENSE-MIT or http://opensource.org/licenses/MIT>, at your\noption.\n",
        "Copyright 2020 Nicolas Silva\n\nLicensed under the Apache License, Version 2.0 (the \"License\");\nyou may not use this file except in compliance with the License.\nYou may obtain a copy of the License at\n\n    http://www.apache.org/licenses/LICENSE-2.0\n\nUnless required by applicable law or agreed to in writing, software\ndistributed under the License is distributed on an \"AS IS\" BASIS,\nWITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.\n",
        "SPDXVersion: SPDX-2.1\nDataLicense: CC0-1.0\nPackageName: muda\n",
    ] {
        assert_eq!(
            classify::classify(statement),
            Class::Statement,
            "{statement}"
        );
    }
}

#[test]
fn a_licence_text_is_never_a_statement_and_a_custom_licence_is_unknown() {
    assert_eq!(classify::classify(MIT), Class::Licenses(list(&["MIT"])));
    assert!(!classify::is_statement(MIT));
    let custom = "Custom licence\n\nYou may use this software for any purpose. The software is provided without warranty of any kind.";
    assert_eq!(classify::classify(custom), Class::Unknown);
    assert_eq!(
        classify::classify("Something else entirely."),
        Class::Unknown
    );
    assert!(!classify::is_statement(""));
}

#[test]
fn normalising_makes_line_ends_trailing_space_and_outer_blank_lines_irrelevant() {
    let a = "\n\n  \nHello  \r\nWorld\t\r\n\n\n";
    let b = "Hello\nWorld";
    assert_eq!(classify::normalize(a), "Hello\nWorld");
    assert_eq!(classify::normalize(b), "Hello\nWorld");
    assert_eq!(classify::normalize("\u{feff}x"), "x");
    assert_eq!(classify::normalize(a), classify::normalize(b));
    // Inner blank lines and indentation are kept.
    assert_eq!(classify::normalize("a\n\n  b"), "a\n\n  b");
}

#[test]
fn a_placeholder_line_is_replaced_and_keeps_its_indentation() {
    let text = classify::with_copyright(common::MIT_STANDARD, "Copyright (c) the objc2 authors");
    assert!(text.contains("\nCopyright (c) the objc2 authors\n"));
    assert!(!text.contains("<year>") && !text.contains("<copyright holders>"));
    assert_eq!(
        text.matches("Copyright").count(),
        1,
        "only the replaced line"
    );

    let apache = classify::with_copyright(common::APACHE_STANDARD, "Copyright (c) 2020 Jane");
    assert!(
        apache.contains("\n   Copyright (c) 2020 Jane\n"),
        "{apache}"
    );
    assert!(!apache.contains("[yyyy]"));
}

#[test]
fn a_text_without_a_placeholder_gets_the_line_above() {
    let text = classify::with_copyright(
        "Mozilla Public License Version 2.0\n\n1. Definitions\n",
        "Copyright (c) the stylo authors",
    );
    assert_eq!(
        text,
        "Copyright (c) the stylo authors\n\nMozilla Public License Version 2.0\n\n1. Definitions"
    );
}

#[test]
fn the_standard_texts_of_the_repository_are_what_the_tool_expects() {
    let licenses = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LICENSES");
    let expected = [
        ("Apache-2.0", "Apache-2.0"),
        ("BSD-2-Clause", "BSD-2-Clause"),
        ("BSD-3-Clause", "BSD-3-Clause"),
        ("BSL-1.0", "BSL-1.0"),
        ("CC0-1.0", "CC0-1.0"),
        ("ISC", "ISC"),
        ("MIT", "MIT"),
        ("MPL-2.0", "MPL-2.0"),
        ("Unicode-3.0", "Unicode-3.0"),
        ("Unlicense", "Unlicense"),
        ("Zlib", "Zlib"),
    ];
    let mut found = 0;
    for (file, id) in expected {
        let Ok(text) = std::fs::read_to_string(licenses.join(format!("{file}.txt"))) else {
            continue;
        };
        found += 1;
        assert!(
            ids(&text).contains(&id.to_owned()),
            "LICENSES/{file}.txt is not recognised as {id}"
        );
    }
    assert!(found >= 1, "LICENSES/MPL-2.0.txt is in the repository");
}
