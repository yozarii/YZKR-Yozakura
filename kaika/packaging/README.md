# Packaging kaika

How to cut a release. Nothing here is automated and nothing is published by default.

1. Bump `version` in `kaika/Cargo.toml` and make sure `cd kaika && cargo test` passes.
2. Commit the bump and create a tag, e.g. `git tag v0.1.0` (push only when you intend to release).
3. Make a source tarball of the tag: `git archive --format=tar.gz --prefix=kaika-0.1.0/ -o kaika-0.1.0.tar.gz v0.1.0`.
4. Compute its checksum: `shasum -a 256 kaika-0.1.0.tar.gz`.
5. Upload the tarball to the GitHub release for that tag so the URL is stable.
6. Copy `kaika.rb` into your tap (e.g. `homebrew-tap/Formula/kaika.rb`).
7. Replace the `url` and `sha256` placeholders with the values from steps 5 and 4.
8. Check the formula: `brew audit --strict --new kaika` (or `brew audit --strict Formula/kaika.rb`), then `brew install --build-from-source kaika` and `brew test kaika`.
9. Commit and push the tap. Users then run `brew install <tap>/kaika`.
10. Tarballs and tags are the only release artifacts; the formula builds from source with `cargo install`.
