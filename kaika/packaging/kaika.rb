# TEMPLATE — not a working formula until the placeholders below are replaced.
# See packaging/README.md for the release steps.
class Kaika < Formula
  desc "Universal package manager for macOS (Apple silicon)"
  homepage "https://github.com/yozarii/YZKR-Yozakura"
  # PLACEHOLDER: replace with the release tarball URL for the tag being shipped.
  url "https://example.invalid/kaika/v0.1.0.tar.gz"
  # PLACEHOLDER: replace with `shasum -a 256` of that exact tarball.
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/kaika --version")
  end
end
