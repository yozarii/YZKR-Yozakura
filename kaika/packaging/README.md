# Packaging kaika

How a release is cut. Nothing here publishes by itself: you push the tag, a workflow builds and attaches the files,
and you update the formula in your tap.

## What a release is

One source tarball, `kaika-<version>.tar.gz`, with its `kaika-<version>.tar.gz.sha256`, attached to the GitHub release
for the tag `v<version>`. It holds the contents of `kaika/` only (not the rest of the repository), under the folder
`kaika-<version>/`, and is made by
`git archive --format=tar --prefix=kaika-<version>/ v<version>:kaika | gzip -n -9`.
`kaika/.gitattributes` leaves `packaging/kaika.rb` out of it, so changing the formula never changes the tarball.

The Homebrew formula builds that tarball with `cargo install` on the user's machine (Apple silicon, `depends_on
"rust" => :build`). There is no prebuilt binary and no universal build; it is a source release.

## Cutting 0.1.0

1. `kaika/Cargo.toml` has the version, `kaika/CHANGELOG.md` has its entry; `cd kaika && cargo test` passes.
2. Tag the commit and push the tag: `git tag v0.1.0 && git push origin v0.1.0`.
   `.github/workflows/kaika.yml` then runs the tests and the release build, checks that the tag equals the
   Cargo.toml version, builds the tarball and checksum, and creates the GitHub release with both attached. It does
   nothing else.
3. Put the published checksum into the formula and copy it into the tap:

   ```bash
   sha=$(gh release download v0.1.0 -R yozarii/YZKR-Yozakura -p '*.sha256' -O - | cut -d' ' -f1)
   sed -i '' "s/^  sha256 .*/  sha256 \"$sha\"/" kaika/packaging/kaika.rb
   ```

4. In the tap (`homebrew-<name>`), put the file at `Formula/kaika.rb`, commit and push. Users then run
   `brew install <owner>/<name>/kaika`.

The checksum in `kaika.rb` in this repository is that of a tarball made locally from the release commit; the one the
workflow publishes can differ (a tarball records the commit's time), which is why step 3 takes it from the release.

## Checking a formula without publishing

```bash
brew style kaika/packaging/kaika.rb
brew tap-new --no-git you/scratch && cp kaika/packaging/kaika.rb "$(brew --repository you/scratch)/Formula/"
brew audit --strict --new you/scratch/kaika
brew install --build-from-source you/scratch/kaika && brew test you/scratch/kaika
brew untap you/scratch
```

For an install from a local tarball, replace the `url` in the tap's copy with `file:///path/to/kaika-0.1.0.tar.gz`.
