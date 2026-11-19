// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! The PDF fixtures in `tests/pages/pdf/` are exactly what the generator in
//! `mercury_pdf::testing` writes, byte for byte, so they can be rebuilt (and reviewed) without
//! any other tool: run this test with `MERCURY_WRITE_FIXTURES=1` to write them.

use std::path::PathBuf;

use mercury_pdf::testing::{DocumentSpec, document_pdf, rotated_spec, three_pages_spec};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/pages/pdf")
        .join(name)
}

fn check(name: &str, spec: &DocumentSpec) {
    let wanted = document_pdf(spec);
    let path = fixture_path(name);
    if std::env::var_os("MERCURY_WRITE_FIXTURES").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &wanted).unwrap();
    }
    let committed = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("{} cannot be read: {error}", path.display()));
    assert_eq!(
        committed, wanted,
        "{name} differs from the generator's output; run this test with MERCURY_WRITE_FIXTURES=1"
    );
    assert!(committed.starts_with(b"%PDF-1.4\n"));
}

#[test]
fn three_pages_pdf_is_the_generators_output() {
    check("three-pages.pdf", &three_pages_spec());
}

#[test]
fn rotated_pdf_is_the_generators_output() {
    check("rotated.pdf", &rotated_spec());
}
