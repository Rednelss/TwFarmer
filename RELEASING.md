# Releasing TwFarmer

**English** | [Русский](RELEASING.ru.md)

## Before tagging

1. Update `version` in `Cargo.toml` and the corresponding `Cargo.lock` package.
2. Update `CHANGELOG.md`, `CHANGELOG.ru.md`, `packaging/RELEASE_NOTES.md`, and
   `packaging/RELEASE_NOTES.ru.md`. Keep both documentation languages current.
3. Run the checks and build:

   ```bash
   cargo fmt --check
   cargo test --locked
   cargo clippy --locked --all-targets -- -D warnings
   cargo build --release --locked
   python3 -m unittest discover -s scripts -p 'test_*.py'
   ```

4. Check `--version`, first setup, sign-in, farming, and switching between
   English and Russian. Automated tests do not confirm actual Twitch points.
5. Review changes before committing. Local configuration, sessions, and control
   files are ignored; do not force-add them.

## GitHub Release

Commit the checked changes and push `main`, then create a tag matching the
package version. For the first release:

```bash
git tag -a v0.1.0 -m "TwFarmer v0.1.0"
git push origin v0.1.0
```

The `Release` workflow runs checks on Linux, Windows, and macOS, checks Rust 1.89
compatibility, and verifies that the tag matches `Cargo.toml`. It builds:

| System | Archive |
| --- | --- |
| Linux x86_64, glibc 2.35+ | `twfarmer-vVERSION-x86_64-unknown-linux-gnu.tar.gz` |
| Windows x86_64 | `twfarmer-vVERSION-x86_64-pc-windows-msvc.zip` |
| macOS Apple Silicon | `twfarmer-vVERSION-aarch64-apple-darwin.tar.gz` |

After a successful build, a **draft** GitHub Release appears with archives,
`SHA256SUMS`, and bilingual notes from `packaging/RELEASE_NOTES.md` and
`packaging/RELEASE_NOTES.ru.md`. Review the draft and attachments, then publish it.
The workflow does not overwrite an existing release with the same tag.

GitHub Actions must be enabled in the repository. Only the draft creation job
receives `contents: write`; no personal access token is needed.
Publishing to crates.io is disabled.

## Local Linux archive

Python 3.11+ and a built binary are required:

```bash
python3 scripts/package_release.py \
  --target x86_64-unknown-linux-gnu \
  --binary target/release/twfarmer
cd dist
sha256sum --check *.sha256
```

The script verifies the binary version and packages only explicitly listed
public files, including English and Russian README and changelog files.
Local configuration and sessions are excluded.
Compatibility of a local Linux build depends on the build system's glibc;
use the GitHub Actions archive for distribution.

To package Windows or macOS, pass the matching `--target` and `--binary` on a
system where that binary can run.
