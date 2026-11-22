# Browser extensions (stage 1: the parts without the engine)

Two library crates carry the first stage of extension support. Neither depends on Servo, Slint or the network, so both build and test in minutes with the other engine-free crates (`cargo test --release -p mercury-dnr -p mercury-extensions`). Nothing in the browser calls them yet: the engine hooks, the extension scheme, the APIs and the interface come in later lanes (EE20 to EE27, EE29 to EE31, EE34 to EE36 in the engine plan).

| Crate | Item | What it does |
| --- | --- | --- |
| `crates/mercury-dnr` | EE28 | The `declarativeNetRequest` evaluator: rule parsing and limits, URL filters, and the verdict (or a request rewrite) for one request |
| `crates/mercury-extensions` | EE32, EE33 | Reading and verifying `.crx` and `.xpi` packages, the manifest, the update-check model, and the permission and security model |

An extension package is untrusted input. Every module follows the same rules: size caps on everything read, paths refused before anything is written, signatures checked before anything is used, and no panic on malformed input (`unwrap`, `expect` and `panic!` are denied outside tests; each parser has a deterministic mutation test). Time is always passed in (`now`); the crates never read the clock, the network or the random number generator.

## Installing a package (`mercury-extensions::install`)

```text
bytes ──verify signature──▶ open archive ──read manifest.json──▶ InstallPlan ──(user agrees)──▶ commit_install
        (crx / xpi)          (caps, paths)    (cap, parse, ids)   trust + warnings               extract_to(new dir)
```

- `plan_crx(bytes, &CrxPolicy, &InstallOptions)` and `plan_xpi(bytes, &InstallOptions)` verify first and return an `InstallPlan`; nothing is written. `plan_unpacked(dir, ..)` is developer mode: no signature, the folder is used in place, and links are refused.
- `InstallPlan` carries the `Trust` (`MozillaSigned`, `StoreSigned`, `DeveloperSigned`, `UnpackedDev`), the parsed `Manifest`, the id, `permission_warnings` for the prompt, and warnings (a missing icon). A missing file the extension cannot start without (a background script, a content script, a DNR ruleset) is an error.
- `commit_install(&plan, dest)` extracts to a directory that does not exist or is empty, and removes what it wrote if anything fails.
- The only archive handle install code uses comes from a `VerifiedCrx` or `VerifiedXpi`. `ZipPackage::open` is public (it is safe on its own: caps and path rules) but it does not check a signature.

### CRX3 (`crx`)

`verify_crx` uses `crx3` 2.0 (Chromium's rules). `CrxPolicy::Developer` requires the developer proof only, which is all a local file can have (decision OD-28: `.crx` drag-and-drop only; the shell shows a warning for `Trust::DeveloperSigned`). `CrxPolicy::store()` pins the Chrome Web Store's production publisher key for a later store flow. Differential updates (`CrOD`) are refused. `verified_contents` is kept as bytes and not checked yet.

### XPI (`xpi`, `roots`)

`verify_xpi` follows Firefox's signature check:

- **COSE** (`META-INF/cose.sig` over `cose.manifest`) with ES256, ES384 or ES512, and **PKCS#7** (`mozilla.rsa`, `mozilla.sf`, `manifest.mf`). Manifest version 3 requires COSE; version 2 accepts either. A signature that is present and fails is an error, never "unsigned".
- The chain must end at Mozilla's add-ons root, which is bundled (`crates/mercury-extensions/data/`, decision OD-29; licence and fingerprints in `data/README.md` and `THIRD-PARTY-NOTICES.md`). The signer's common name must be the add-on id.
- Every file of the archive must be listed in the signed manifest with a matching digest, and every listed file must exist.
- Certificate dates: Firefox ignores them for add-on signatures, because Mozilla's end-entity certificates are short-lived. `InstallOptions` does the same (`ValidityPolicy::IgnoreExpiry`); `ValidityPolicy::Enforce` is available.
- RSA and ECDSA primitives come from `aws-lc-rs`; `x509-cert`, `cms` and `coset` only parse. A test PKI covers the mutations; one real Mozilla-signed add-on (`tests/fixtures/`) proves the check against the bundled root.

### Archive limits (`archive::Limits`)

Package 200 MiB; 20,000 entries; one entry 100 MiB and all entries 500 MiB uncompressed; an entry over 1 MiB that compresses more than 200:1 is refused; names at most 512 bytes and 32 deep; manifest at most 1 MiB. Declared sizes are not trusted: the bytes actually inflated are counted. Names with `..`, absolute paths, backslashes, NUL or control characters, duplicates, names that collide when case-folded, symbolic links and encrypted entries are refused, and extraction uses `create_new`, never follows links and re-checks the final path.

## The manifest, permissions and updates

- `manifest`: `Manifest::parse` reads manifest versions 2 and 3 with Chrome and Firefox keys into a typed model, with precise errors, warnings (MV2-only keys in MV3) and the list of files it points at. Nesting is pre-scanned before JSON parsing.
- `match_pattern`: `MatchPattern` and `MatchPatternSet`, written from the MDN and Chrome pages (`webextension_pattern` has no `covers` or `overlaps`). The scheme `*` means `http` and `https` only, and `<all_urls>` excludes `data` and `urn`: the narrow reading.
- `permissions` (EE33): the permission names with their placement and install warnings (Chrome's wording and combination rules), `PermissionSet`, site access (`OnClick`, specific sites, all sites; broad hosts start withheld), `activeTab` grants (given only with a `UserGesture`; ended by any cross-document navigation or tab close), optional-permission requests (gesture required, only what is declared), the private-window opt-in, the kill switch with a crash breaker, and the update consent check (`permissions_added`, `update_needs_consent`).
- **`can_access`** is the single check that injection (EE21), privileged fetch (EE30) and `webRequest` (EE29) must call. It denies the surfaces extensions never touch: `mercury:` and `about:` pages, other extensions' pages, `view-source:`, `data:` and `blob:` top-level documents, the extension stores' pages, and `file:` unless the user allowed it for that extension.
- `update` (decision OD-32): `build_update2_request`, `parse_update2_response` (Omaha update2 XML; DOCTYPE and entity tricks are refused), `parse_firefox_update_manifest`, `judge_update` (https only, no downgrade, no credentials in the URL, matching id) and `check_for_updates` over an `UpdateTransport` trait that the shell implements. Tests use a fake transport; no test touches the network. Mercury does not call the Chrome Web Store (`store_update_url` is tested and unused).

## `declarativeNetRequest` (`mercury-dnr`)

`parse_ruleset(json, RulesetKind::{Static, Dynamic, Session}, &Limits)` returns the rules and warnings. An invalid static rule is skipped with a warning (Chrome's behaviour); an invalid dynamic or session rule rejects the update. `Evaluator::new(rulesets, extension_base)` builds a token index; `evaluate(&RequestMetadata, &FrameState)` returns a `Verdict`: `None`, `Allow`, `AllowAllRequests`, `Block`, `Upgrade`, `Redirect` or `ModifyHeaders`. Highest priority wins; at one priority the order is allow, allowAllRequests, block, upgradeScheme, redirect. `merge_verdicts` combines extensions the way Chromium does (most recently installed first, a tie goes to the newer one).

- `RequestMetadata` has the net-1 request-metadata fields (EC38): `url`, `method`, `resource_type`, `initiator_origin`, `top_level_site`, `is_third_party` and the tab id. `is_third_party: None` is treated as third party (fail closed).
- `Verdict::rewrite()` gives the shape of net-1's `WebResourceRewrite` (EC37): `url`, `set_headers`, `remove_headers`. `append` request operations and every response-header operation are returned in `ModifyHeaders` only: net-1's rewrite has no place for them.
- **Limits**: Chrome's numbers (30,000 guaranteed static rules, 330,000 global, 50 enabled static rulesets, 30,000 dynamic, 5,000 unsafe dynamic, 5,000 session, 1,000 regex rules) plus Mercury's caps on the ruleset size (16 MiB), one rule (16 KiB), `urlFilter` and `regexFilter` (2,048 bytes), a compiled regex (2 MiB, DFA 2 MiB) and header operations per rule. `regexFilter` uses the `regex` crate (linear time; no back-references or look-around, as in RE2); the differences from RE2 are listed in `url_filter.rs`.
- Conditions the evaluator does not implement are refused (the rule is invalid), not ignored, so a rule never matches more than its author wrote.
- The index is checked against a linear scan on more than 20,000 generated cases.

### What is left for the other lanes

- The engine hook in `talaria-servo`'s `load_web_resource` (needs net-1 on `mercury`): build `RequestMetadata` from `WebResourceRequest`, run Mercury's own blocker first (decision OD-35: an extension `allow` cannot override it), map `Block` to a cancel and `Verdict::rewrite()` to `WebResourceResponseMsg::Rewrite`, and keep `AllowAllRequests` grants per frame (needs the frame ids of EE31).
- A response-header hook for `modifyHeaders` on responses.
- `getMatchedRules`, the badge counter, storage of rulesets, reading `declarative_net_request.rule_resources` files (the manifest lists them; `install` checks they exist).
- The extension scheme and web-accessible resources (EE23), the `chrome.*` API surface (EE25), background pages (EE26), popups (EE27), `contextMenus` (EE34) and native messaging (EE35).
- `verified_contents` checking, the Chrome Web Store install flow (OD-28), the update scheduler, and the install and permission prompts.
