// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors
//! A fake repository in a temporary directory: the files `mercury-credits` reads, a few packages
//! with licence files, and the Cargo output (tree and metadata) as values, so that no cargo runs.
#![allow(dead_code)]
use mercury_credits::app::{self, Outcome};
use mercury_credits::cargo::{Metadata, Package};
use mercury_credits::collect::Inputs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

pub const MIT: &str = "MIT License

Copyright (c) 2019 Jane Roe

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";

/// The standard text, with the placeholder line of the SPDX template.
pub const MIT_STANDARD: &str = "MIT License

Copyright (c) <year> <copyright holders>

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";

/// The start and the appendix of the Apache licence (the wording the classifier looks for).
pub const APACHE: &str = "                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

      \"License\" shall mean the terms and conditions for use, reproduction,
      and distribution as defined by Sections 1 through 9 of this document.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

   Copyright 2020 Jane Roe

   Licensed under the Apache License, Version 2.0 (the \"License\");
   you may not use this file except in compliance with the License.
";

pub const APACHE_STANDARD: &str = "                                 Apache License
                           Version 2.0, January 2004
                        http://www.apache.org/licenses/

   TERMS AND CONDITIONS FOR USE, REPRODUCTION, AND DISTRIBUTION

   1. Definitions.

   END OF TERMS AND CONDITIONS

   APPENDIX: How to apply the Apache License to your work.

   Copyright [yyyy] [name of copyright owner]

   Licensed under the Apache License, Version 2.0 (the \"License\");
";

pub const MPL: &str = "Mozilla Public License Version 2.0
==================================

1. Definitions
--------------

1.1. \"Contributor\" means each individual or legal entity that creates,
contributes to the creation of, or owns Covered Software.
";

pub const ISC: &str = "ISC License

Copyright (c) 2015, Some One

Permission to use, copy, modify, and/or distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED \"AS IS\" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS.
";

pub const BSD3: &str = "Copyright (c) 2016, Some One
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:

1. Redistributions of source code must retain the above copyright notice, this
   list of conditions and the following disclaimer.

2. Redistributions in binary form must reproduce the above copyright notice,
   this list of conditions and the following disclaimer in the documentation
   and/or other materials provided with the distribution.

3. Neither the name of the copyright holder nor the names of its
   contributors may be used to endorse or promote products derived from
   this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\".
";

pub const DENY: &str = r#"
[licenses]
allow = [
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "MIT",
    "MPL-2.0",
    "Unicode-3.0",
    "Zlib",
]
confidence-threshold = 0.9

[[licenses.exceptions]]
crate = "slinty"
allow = ["LicenseRef-Slint-Royalty-free-2.0"]
"#;

pub struct Fake {
    pub dir: TempDir,
    pub packages: Vec<Package>,
    tree: Vec<String>,
    members: Vec<String>,
}

fn id_of(source: Option<&str>, name: &str, version: &str, path: &Path) -> String {
    match source {
        Some(_) => {
            format!("registry+https://github.com/rust-lang/crates.io-index#{name}@{version}")
        }
        None => format!("path+file://{}#{version}", path.display()),
    }
}

impl Fake {
    pub fn new() -> Self {
        let fake = Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
            packages: Vec::new(),
            tree: Vec::new(),
            members: Vec::new(),
        };
        fake.write(
            "Cargo.toml",
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"9.9.9\"\n",
        );
        fake.write("Cargo.lock", "# a lock file\n");
        fake.write("NOTICE", "Mercury Browser notice\nsecond line\n");
        fake.write(
            "THIRD-PARTY-NOTICES.md",
            "# Third-party notices\n\nBundled libraries.\n",
        );
        fake.write("deny.toml", DENY);
        fake.write("LICENSES/MIT.txt", MIT_STANDARD);
        fake.write("LICENSES/Apache-2.0.txt", APACHE_STANDARD);
        fake.write("LICENSES/MPL-2.0.txt", MPL);
        fake.write("LICENSES/ISC.txt", ISC);
        let mut fake = fake;
        fake.add_own("app", "9.9.9");
        fake
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, relative: &str, text: &str) {
        let path = self.root().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn add_package(
        &mut self,
        name: &str,
        version: &str,
        license: Option<&str>,
        directory: PathBuf,
        registry: bool,
    ) {
        std::fs::create_dir_all(&directory).unwrap();
        let source =
            registry.then(|| "registry+https://github.com/rust-lang/crates.io-index".to_owned());
        self.packages.push(Package {
            id: id_of(source.as_deref(), name, version, &directory),
            name: name.to_owned(),
            version: version.to_owned(),
            source,
            license: license.map(str::to_owned),
            license_file: None,
            authors: Vec::new(),
            repository: None,
            manifest_path: directory.join("Cargo.toml"),
            links: None,
        });
        let location = if registry {
            String::new()
        } else {
            format!(" ({})", directory.display())
        };
        self.tree.push(format!("{name} v{version}{location}"));
    }

    /// A crate from crates.io, in `<root>/registry/<name>-<version>/`.
    pub fn add(&mut self, name: &str, version: &str, license: &str) -> &mut Self {
        let directory = self
            .root()
            .join("registry")
            .join(format!("{name}-{version}"));
        self.add_package(name, version, Some(license), directory, true);
        self
    }

    /// A registry crate that declares no `license`.
    pub fn add_without_license(&mut self, name: &str, version: &str) -> &mut Self {
        let directory = self
            .root()
            .join("registry")
            .join(format!("{name}-{version}"));
        self.add_package(name, version, None, directory, true);
        self
    }

    /// A workspace crate.
    pub fn add_own(&mut self, name: &str, version: &str) -> &mut Self {
        let directory = self.root().join("crates").join(name);
        self.add_package(
            name,
            version,
            Some("PolyForm-Noncommercial-1.0.0"),
            directory,
            false,
        );
        let id = self.packages.last().unwrap().id.clone();
        self.members.push(id);
        self
    }

    /// A crate under `third_party/<name>`, not a workspace member.
    pub fn add_vendored(&mut self, name: &str, version: &str, license: &str) -> &mut Self {
        let directory = self.root().join("third_party").join(name);
        self.add_package(name, version, Some(license), directory, false);
        self
    }

    pub fn package(&mut self, name: &str) -> &mut Package {
        self.packages
            .iter_mut()
            .find(|package| package.name == name)
            .unwrap_or_else(|| panic!("no package {name}"))
    }

    pub fn dir_of(&self, name: &str) -> PathBuf {
        self.packages
            .iter()
            .find(|package| package.name == name)
            .unwrap_or_else(|| panic!("no package {name}"))
            .root()
            .to_owned()
    }

    /// A file inside a package.
    pub fn crate_file(&self, name: &str, relative: &str, text: &str) {
        let path = self.dir_of(name).join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// The tree text, with `(*)` repeats and a proc-macro marker added like Cargo does.
    pub fn tree_text(&self) -> String {
        let mut text = self.tree.join("\n");
        text.push('\n');
        text
    }

    pub fn metadata(&self) -> Metadata {
        Metadata {
            packages: self.packages.clone(),
            workspace_members: self.members.clone(),
        }
    }

    pub fn outcome(&self) -> Outcome {
        let metadata = self.metadata();
        let tree = self.tree_text();
        app::analyse_inputs(&Inputs {
            root: self.root(),
            tree: &tree,
            metadata: &metadata,
            target: "aarch64-apple-darwin",
            runtime_only: false,
        })
    }
}
