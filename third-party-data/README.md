# Third-party data licenses

Full license texts and attribution for the third-party **datasets** vendored as
test fixtures in this repository. The data itself and its curation/provenance
record live next to the test that uses it:
`crates/aionforge-security/tests/corpus/` (see that directory's `PROVENANCE.md`).

These license texts are kept here, outside any crate's package directory, on
purpose: `cargo-about` (which generates `THIRDPARTY.md` from `Cargo.lock`)
content-scans each crate's tree for license files, and a license file placed
inside `crates/aionforge-security/` would be misattributed as that first-party
crate's own license. The repo root is a virtual workspace and is not scanned, so
the attribution stays correct.

| File | Dataset | License |
|------|---------|---------|
| `deepset-prompt-injections.LICENSE.txt` | [deepset/prompt-injections](https://huggingface.co/datasets/deepset/prompt-injections) | Apache-2.0 |
| `NotInject.LICENSE.txt` | [leolee99/NotInject](https://huggingface.co/datasets/leolee99/NotInject) | MIT |

These govern the vendored test data only; the aionforge-memory source is licensed
`MIT OR Apache-2.0` (see the root `LICENSE-MIT` / `LICENSE-APACHE`).
