# Bundled trust anchors for add-on signatures

These three files are Mozilla's add-ons **production** signing certificates. They are public data,
copied byte for byte (no re-encoding, no edits) and embedded by `src/roots.rs` with `include_bytes!`.

| File | Subject | Valid until | SHA-256 fingerprint (of the DER) |
| --- | --- | --- | --- |
| `addons-public.pem` | `CN=root-ca-production-amo` (self-signed root) | 3 Dec 2200 | `C8:A8:0E:9A:FA:EF:4E:21:9B:6F:B5:D7:A7:1D:0F:10:12:23:BA:C5:00:1A:C2:8F:9B:0D:43:DC:59:A1:06:DB` |
| `addons-public-intermediate.pem` | `CN=signingca1.addons.mozilla.org` | 3 Dec 2100 | `16:76:1B:C9:7F:84:D1:4E:FB:88:8F:36:FC:92:3A:97:18:65:AE:00:1C:92:88:FC:4F:A4:72:12:95:5B:F4:A2` |
| `addons-public-2018-intermediate.pem` | `CN=production-signing-ca.addons.mozilla.org` | 19 Mar 2125 | `70:C7:FA:72:BC:3C:71:B3:0E:7D:45:76:2B:92:61:B9:D8:39:54:4B:35:C4:3F:27:F7:F3:9D:9C:EA:1E:31:D0` |

The fingerprints are pinned by tests in `src/roots.rs`; compute them yourself with
`openssl x509 -in <file> -noout -fingerprint -sha256`.

## Source

- Repository: `mozilla-firefox/firefox` (mozilla-central).
- Directory: `security/manager/ssl/`, files `addons-public.pem`, `addons-public-intermediate.pem` and
  `addons-public-2018-intermediate.pem`.
- Commit: `e27b32bb75ad036200712c983ed5f2c28d069def` (the tip of the repository on 22 Nov 2026, the day the files were copied; the clone was shallow, so this is the commit they were read at, not the commit that last changed them).
- Firefox compiles these into `addonsPublicRoots` and `addonsPublicIntermediates`
  (`security/manager/ssl/moz.build`) and uses them in `AppTrustDomain.cpp`: the root is the only trust
  anchor and the intermediates are offered as path-building candidates.

The stage root (`addons-stage.pem`) is **not** bundled.

## Licence

The files live in Firefox's source tree, which is licensed under the Mozilla Public License 2.0. The
certificates are public data that Mozilla publishes so that any client can verify its signatures. This
README makes no claim beyond that: the files keep whatever terms Mozilla's tree gives them, and
`THIRD-PARTY-NOTICES.md` records the source.

## How to update

1. Check out the new commit of `mozilla-firefox/firefox` (a sparse checkout of `security/manager/ssl`
   is enough) and copy the three files over byte for byte.
2. Recompute the fingerprints with `openssl x509 -in <file> -noout -fingerprint -sha256 -dates`, update
   this table and the three pinned values in `src/roots.rs`.
3. Update the commit above and run `cargo test -p mercury-extensions` (the real-add-on test in
   `tests/package_xpi_real.rs` must still verify against the bundled root).
